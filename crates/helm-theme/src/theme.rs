//! Apply, diff and lint: the three things `helm ctl theme` does.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use helm_core::palette::Finding;
use helm_core::Palette;

use crate::render::render_derived;
use crate::{templates, Error, Reload, Reloader, Result, SystemReloader, Template};

/// The palette helm ships, embedded so a first run has something to copy.
pub const SHIPPED_PALETTE: &str = include_str!("../../../palette.toml");

/// Where the user's palette lives, relative to `$XDG_CONFIG_HOME`.
pub const USER_PALETTE: &str = "helm/palette.toml";

/// Suffix of the file each output is staged in before it is renamed into place.
const STAGING_SUFFIX: &str = ".helm-tmp";

/// What one apply did.
#[derive(Debug)]
pub struct Applied {
    /// Files whose contents changed and were renamed into place.
    pub written: Vec<PathBuf>,
    /// Byte-identical files: not rewritten, not reloaded.
    pub unchanged: Vec<PathBuf>,
    /// Reload mechanisms that fired, named by the first template owning each.
    pub reloaded: Vec<&'static str>,
    /// Wall time for the whole apply. Budget: < 150 ms (ARCHITECTURE.md §4).
    pub elapsed: Duration,
}

/// What one template would do to disk, without doing it.
#[derive(Debug)]
pub struct Change {
    /// Template id.
    pub id: &'static str,
    /// Absolute target path.
    pub target: PathBuf,
    /// True when the rendered bytes differ from what is on disk.
    pub changed: bool,
}

/// Hue distance between two accents, in OKLab degrees.
#[derive(Debug)]
pub struct HueSeparation {
    /// First accent's name.
    pub a: &'static str,
    /// Second accent's name.
    pub b: &'static str,
    /// Absolute separation in degrees.
    pub degrees: f32,
}

/// Everything `helm ctl theme lint` prints.
#[derive(Debug)]
pub struct LintReport {
    /// Findings from [`Palette::lint`], in order.
    pub findings: Vec<Finding>,
    /// Every accent pair and how far apart their hues are.
    pub separations: Vec<HueSeparation>,
}

impl LintReport {
    /// True when nothing fatal stands in the way of applying this palette.
    pub fn is_clean(&self) -> bool {
        !self.findings.iter().any(|f| f.fatal)
    }
}

/// Read the user's palette, seeding it from the shipped one on first run.
///
/// The copy happens before the read so a user's first edit is to their own
/// file, not to something inside the helm package.
pub fn load_palette(root: &Path) -> Result<Palette> {
    reject_symlinked_palette_path(root)?;
    let path = root.join(USER_PALETTE);
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        std::fs::write(&path, SHIPPED_PALETTE).map_err(|e| Error::io(&path, e))?;
    }
    Ok(Palette::load(&path)?)
}

/// Refuse the palette path when it crosses a pre-existing link. Seeding a
/// palette is a write just like rendering a template, so it must not turn a
/// seemingly private configuration root into a write primitive elsewhere.
fn reject_symlinked_palette_path(root: &Path) -> Result<()> {
    if std::fs::symlink_metadata(root)
        .map_err(|error| Error::io(root, error))?
        .file_type()
        .is_symlink()
    {
        return Err(Error::UnsafeTarget {
            target: root.to_path_buf(),
            reason: "configuration root is a symlink",
        });
    }

    let mut path = root.to_path_buf();
    for component in Path::new(USER_PALETTE).components() {
        let Component::Normal(component) = component else {
            unreachable!("USER_PALETTE is a normalized relative path");
        };
        path.push(component);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::UnsafeTarget {
                    target: path,
                    reason: "palette path traverses a symlink",
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(Error::io(&path, error)),
        }
    }
    Ok(())
}

/// Render every shipped template and swap them in as one step.
pub fn apply(palette: &Palette, root: &Path) -> Result<Applied> {
    apply_with(palette, root, &templates(), &mut SystemReloader)
}

/// [`apply`], with the template set and the reload fan-out injected.
///
/// The seam exists for the tests: counting reloads and pointing a whole apply
/// at a temporary directory is the only way to assert atomicity without a
/// desktop underneath it.
pub fn apply_with(
    palette: &Palette,
    root: &Path,
    templates: &[Template],
    reloader: &mut dyn Reloader,
) -> Result<Applied> {
    let started = Instant::now();

    validate_targets(templates)?;
    reject_symlinked_targets(root, templates)?;

    // Lint before rendering, not after writing: a palette that fails its own
    // readability floors must leave the live theme exactly as it was.
    let findings = palette.lint();
    if findings.iter().any(|f| f.fatal) {
        return Err(Error::PaletteRefused(findings));
    }

    // Contrast is folded in once, here, and every template sees the result.
    let derived = palette.derived();

    // Phase 1: everything to memory. A template that will not render must not
    // be discovered halfway through writing the set.
    let mut rendered = Vec::with_capacity(templates.len());
    for t in templates {
        let text = render_derived(&derived, t.id, t.source)?;
        rendered.push((t, root.join(&t.target), text));
    }

    // Phase 2: stage the ones that differ.
    let mut unchanged = Vec::new();
    let mut staged: Vec<(&Template, PathBuf, PathBuf)> = Vec::new();
    for (t, target, text) in &rendered {
        if std::fs::read(target).is_ok_and(|old| old == text.as_bytes()) {
            unchanged.push(target.clone());
            continue;
        }
        match stage(target, text) {
            Ok(tmp) => staged.push((t, tmp, target.clone())),
            Err(e) => {
                discard(&staged);
                return Err(e);
            }
        }
    }

    // Phase 3: rename, which is atomic per file on the same filesystem.
    let mut written = Vec::with_capacity(staged.len());
    for (i, (_, tmp, target)) in staged.iter().enumerate() {
        if let Err(e) = std::fs::rename(tmp, target) {
            discard(&staged[i..]);
            return Err(Error::io(target, e));
        }
        written.push(target.clone());
    }

    // Phase 4: and only now, one reload per distinct mechanism. Reloading per
    // file would show the desktop a half-applied theme, which is the whole
    // reason for the three phases above.
    let mut fired: Vec<&Reload> = Vec::new();
    let mut reloaded = Vec::new();
    for (t, _, _) in &staged {
        if t.reload == Reload::None || fired.contains(&&t.reload) {
            continue;
        }
        reloader.reload(&t.reload)?;
        fired.push(&t.reload);
        reloaded.push(t.id);
    }

    Ok(Applied {
        written,
        unchanged,
        reloaded,
        elapsed: started.elapsed(),
    })
}

/// Report what an apply would change, writing nothing.
pub fn diff(palette: &Palette, root: &Path) -> Result<Vec<Change>> {
    let templates = templates();
    validate_targets(&templates)?;
    let derived = palette.derived();
    let mut out = Vec::new();
    for t in templates {
        let target = root.join(&t.target);
        let text = render_derived(&derived, t.id, t.source)?;
        let changed = !std::fs::read(&target).is_ok_and(|old| old == text.as_bytes());
        out.push(Change {
            id: t.id,
            target,
            changed,
        });
    }
    Ok(out)
}

/// Reject targets which cannot be represented as one unambiguous path below
/// the caller's configuration root. This validation is deliberately before
/// rendering or staging so one malicious template cannot partly apply a set.
fn validate_targets(templates: &[Template]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for template in templates {
        if template.target.as_os_str().is_empty() {
            return Err(Error::UnsafeTarget {
                target: template.target.clone(),
                reason: "target is empty",
            });
        }
        if template.target.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_)
                    | Component::RootDir
                    | Component::CurDir
                    | Component::ParentDir
            )
        }) {
            return Err(Error::UnsafeTarget {
                target: template.target.clone(),
                reason: "target must be a normalized relative path",
            });
        }
        if !seen.insert(template.target.clone()) {
            return Err(Error::UnsafeTarget {
                target: template.target.clone(),
                reason: "two templates target the same output",
            });
        }
    }
    Ok(())
}

/// Refuse a pre-existing link anywhere in an output path. This closes the
/// ordinary user-config symlink escape before staging; descriptor-relative
/// operations provide the remaining race protection in the writer itself.
fn reject_symlinked_targets(root: &Path, templates: &[Template]) -> Result<()> {
    if std::fs::symlink_metadata(root)
        .map_err(|error| Error::io(root, error))?
        .file_type()
        .is_symlink()
    {
        return Err(Error::UnsafeTarget {
            target: root.to_path_buf(),
            reason: "configuration root is a symlink",
        });
    }
    for template in templates {
        let mut path = root.to_path_buf();
        for component in template.target.components() {
            let Component::Normal(component) = component else {
                unreachable!("validate_targets accepted a non-normal target");
            };
            path.push(component);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(Error::UnsafeTarget {
                        target: template.target.clone(),
                        reason: "target traverses a symlink",
                    });
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(Error::io(&path, error)),
            }
        }
    }
    Ok(())
}

/// Lint a palette and measure its accent hue separations.
///
/// The separations are reported whether or not they are in breach: "how close
/// are these two?" is the question a palette author is actually asking.
pub fn lint(palette: &Palette) -> LintReport {
    let accents = palette.derived().accent.named();
    let mut separations = Vec::new();
    for (i, (an, a)) in accents.iter().enumerate() {
        for (bn, b) in &accents[i + 1..] {
            let mut degrees = (a.hue_degrees() - b.hue_degrees()).abs();
            if degrees > 180.0 {
                degrees = 360.0 - degrees;
            }
            separations.push(HueSeparation {
                a: an,
                b: bn,
                degrees,
            });
        }
    }
    LintReport {
        findings: palette.lint(),
        separations,
    }
}

/// Write `text` beside `target` and flush it to the disk.
///
/// The `fsync` is what makes the rename meaningful: without it the directory
/// entry can land before the bytes do, and a power cut mid-apply leaves a
/// correctly named empty stylesheet.
fn stage(target: &Path, text: &str) -> Result<PathBuf> {
    use rustix::fs::{openat, Mode, OFlags, CWD};
    use std::io::Write;

    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(STAGING_SUFFIX);
    let tmp = PathBuf::from(tmp);

    // `CREATE | EXCL | NOFOLLOW` makes a leftover or attacker-provided staging
    // name fail closed instead of following it and clobbering its destination.
    let fd = openat(
        CWD,
        &tmp,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|e| Error::io(&tmp, e.into()))?;
    let mut f = std::fs::File::from(fd);
    f.write_all(text.as_bytes())
        .map_err(|e| Error::io(&tmp, e))?;
    f.sync_all().map_err(|e| Error::io(&tmp, e))?;
    Ok(tmp)
}

/// Remove staged files after a failure, so a retry does not trip over them.
fn discard(staged: &[(&Template, PathBuf, PathBuf)]) {
    for (_, tmp, _) in staged {
        let _ = std::fs::remove_file(tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{templates, Reload, Template};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn shipped() -> Palette {
        Palette::from_toml(SHIPPED_PALETTE).expect("shipped palette must parse")
    }

    /// A reloader that fires nothing and remembers everything, so a test can
    /// assert on the fan-out without a desktop underneath it.
    #[derive(Default)]
    struct Recorder(Vec<Reload>);

    impl Reloader for Recorder {
        fn reload(&mut self, reload: &Reload) -> Result<()> {
            self.0.push(reload.clone());
            Ok(())
        }
    }

    /// Every file under `dir`, as (relative path, bytes). Comparing two of
    /// these is how the atomicity criteria are asserted: a failed apply must
    /// leave the tree byte-for-byte as it found it.
    fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(dir).unwrap().to_path_buf();
                    out.insert(rel, std::fs::read(&path).unwrap());
                }
            }
        }
        out
    }

    #[test]
    fn apply_writes_every_template_with_no_unexpanded_placeholders() {
        let root = tempfile::tempdir().unwrap();
        let set = templates();
        let applied = apply_with(&shipped(), root.path(), &set, &mut Recorder::default()).unwrap();

        assert_eq!(applied.written.len(), set.len());
        assert!(applied.unchanged.is_empty());
        for t in &set {
            let path = root.path().join(&t.target);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(
                !text.contains("{{"),
                "{} left an unexpanded placeholder",
                t.id
            );
            assert!(!text.is_empty(), "{} rendered empty", t.id);
        }
    }

    #[test]
    fn a_second_apply_changes_nothing_and_reloads_nothing() {
        let root = tempfile::tempdir().unwrap();
        let set = templates();
        let p = shipped();
        apply_with(&p, root.path(), &set, &mut Recorder::default()).unwrap();

        let before = tree(root.path());
        let mut second = Recorder::default();
        let applied = apply_with(&p, root.path(), &set, &mut second).unwrap();

        assert!(applied.written.is_empty(), "{:?}", applied.written);
        assert_eq!(applied.unchanged.len(), set.len());
        assert!(applied.reloaded.is_empty());
        assert!(second.0.is_empty(), "a no-op apply reloaded {:?}", second.0);
        assert_eq!(before, tree(root.path()));
    }

    #[test]
    fn only_outputs_referencing_the_changed_colour_are_rewritten() {
        // A purpose-built pair rather than the shipped set: nearly every shipped
        // template mentions the focus accent, which would make the assertion
        // true without testing anything.
        let set = vec![
            Template {
                id: "violet-user",
                source: "focus = {{ accent.violet }}\n",
                target: PathBuf::from("violet.conf"),
                reload: Reload::None,
            },
            Template {
                id: "teal-user",
                source: "ok = {{ accent.teal }}\n",
                target: PathBuf::from("teal.conf"),
                reload: Reload::None,
            },
        ];
        let root = tempfile::tempdir().unwrap();
        let mut p = shipped();
        apply_with(&p, root.path(), &set, &mut Recorder::default()).unwrap();

        p.accent.violet = helm_core::color::Rgb::new(0xb0, 0x7a, 0xff);
        let applied = apply_with(&p, root.path(), &set, &mut Recorder::default()).unwrap();

        assert_eq!(applied.written, vec![root.path().join("violet.conf")]);
        assert_eq!(applied.unchanged, vec![root.path().join("teal.conf")]);
    }

    #[test]
    fn outputs_carry_derived_colours_not_source_literals() {
        let mut p = shipped();
        p.contrast = 1.30;
        let derived = p.derived();
        assert_ne!(derived.accent.violet, p.accent.violet, "1.30 must move it");

        for t in templates() {
            let out = crate::render(&p, t.id, t.source).unwrap();
            for (label, source) in [
                ("accent.violet", p.accent.violet),
                ("text.normal", p.text.normal),
            ] {
                assert!(
                    !out.contains(&source.hex()) && !out.contains(&source.hex_bare()),
                    "{}: {label} is the source literal, so contrast was not folded in",
                    t.id
                );
            }
        }

        // The loop above would also pass on templates that never mention an
        // accent at all, so pin the positive half too.
        let all: String = templates()
            .iter()
            .map(|t| crate::render(&p, t.id, t.source).unwrap())
            .collect();
        assert!(all.contains(&derived.accent.violet.hex_bare()));
        assert!(all.contains(&derived.text.normal.hex_bare()));
    }

    #[test]
    fn an_unknown_placeholder_aborts_the_apply_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut set = templates();
        set.push(Template {
            id: "invented",
            source: "colour = {{ accent.mauve }}\n",
            target: PathBuf::from("invented.conf"),
            reload: Reload::None,
        });

        let err = apply_with(&shipped(), root.path(), &set, &mut Recorder::default())
            .expect_err("an unknown placeholder must abort the apply");

        let msg = err.to_string();
        assert!(msg.contains("accent.mauve"), "unhelpful error: {msg}");
        assert!(
            tree(root.path()).is_empty(),
            "a failed apply left files behind: {:?}",
            tree(root.path()).keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn unsafe_or_duplicate_targets_abort_before_any_output_is_written() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let unsafe_sets = [
            vec![Template {
                id: "absolute",
                source: "x = {{ accent.violet }}\n",
                target: outside.path().join("escaped.conf"),
                reload: Reload::None,
            }],
            vec![
                Template {
                    id: "first",
                    source: "x = {{ accent.violet }}\n",
                    target: PathBuf::from("same.conf"),
                    reload: Reload::None,
                },
                Template {
                    id: "second",
                    source: "x = {{ accent.teal }}\n",
                    target: PathBuf::from("same.conf"),
                    reload: Reload::None,
                },
            ],
        ];

        for set in unsafe_sets {
            let err = apply_with(&shipped(), root.path(), &set, &mut Recorder::default())
                .expect_err("unsafe targets must be rejected");
            assert!(err.to_string().contains("target"), "{err}");
            assert!(tree(root.path()).is_empty());
            assert!(!outside.path().join("escaped.conf").exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_output_parent_is_refused_without_touching_its_destination() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("linked")).unwrap();
        let set = [Template {
            id: "linked",
            source: "x = {{ accent.violet }}\n",
            target: PathBuf::from("linked/escaped.conf"),
            reload: Reload::None,
        }];

        let err = apply_with(&shipped(), root.path(), &set, &mut Recorder::default())
            .expect_err("a symlinked output parent must be refused");
        assert!(err.to_string().contains("symlink"), "{err}");
        assert!(!outside.path().join("escaped.conf").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_configuration_root_is_refused() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = parent.path().join("config");
        symlink(outside.path(), &root).unwrap();
        let set = [Template {
            id: "root-link",
            source: "x = {{ accent.violet }}\n",
            target: PathBuf::from("theme.conf"),
            reload: Reload::None,
        }];

        let err = apply_with(&shipped(), &root, &set, &mut Recorder::default())
            .expect_err("a symlinked configuration root must be refused");
        assert!(err.to_string().contains("root"), "{err}");
        assert!(!outside.path().join("theme.conf").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_staging_file_is_refused_without_touching_its_destination() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim");
        std::fs::write(&victim, "do not overwrite").unwrap();
        symlink(&victim, root.path().join("theme.conf.helm-tmp")).unwrap();
        let set = [Template {
            id: "staging-link",
            source: "x = {{ accent.violet }}\n",
            target: PathBuf::from("theme.conf"),
            reload: Reload::None,
        }];

        let err = apply_with(&shipped(), root.path(), &set, &mut Recorder::default())
            .expect_err("a symlinked staging file must be refused");
        assert!(err.to_string().contains("helm-tmp"), "{err}");
        assert_eq!(std::fs::read_to_string(victim).unwrap(), "do not overwrite");
    }

    #[test]
    fn a_fatally_linted_palette_is_refused_and_leaves_the_theme_live() {
        let root = tempfile::tempdir().unwrap();
        let set = templates();
        apply_with(&shipped(), root.path(), &set, &mut Recorder::default()).unwrap();
        let live = tree(root.path());

        let mut bad = shipped();
        bad.text.normal = bad.background.pane; // fatal: body text on its own background
        let err = apply_with(&bad, root.path(), &set, &mut Recorder::default())
            .expect_err("a fatally linted palette must be refused");

        match &err {
            crate::Error::PaletteRefused(findings) => {
                assert!(findings.iter().any(|f| f.fatal && f.path == "text.normal"))
            }
            other => panic!("wrong error: {other}"),
        }
        assert_eq!(live, tree(root.path()), "the live theme was disturbed");
    }

    #[test]
    fn each_reload_mechanism_fires_exactly_once() {
        let root = tempfile::tempdir().unwrap();
        let set = templates();
        let mut rec = Recorder::default();
        let applied = apply_with(&shipped(), root.path(), &set, &mut rec).unwrap();

        let mut distinct: Vec<&Reload> = Vec::new();
        for t in &set {
            if t.reload != Reload::None && !distinct.contains(&&t.reload) {
                distinct.push(&t.reload);
            }
        }
        assert!(
            distinct.len() < set.len(),
            "this test only means something when templates share a mechanism"
        );
        assert_eq!(rec.0.len(), distinct.len(), "fired {:?}", rec.0);
        assert_eq!(applied.reloaded.len(), distinct.len());
        for r in distinct {
            assert_eq!(rec.0.iter().filter(|f| *f == r).count(), 1, "{r:?}");
        }
    }

    #[test]
    fn first_run_copies_the_shipped_palette_to_the_user_config() {
        let root = tempfile::tempdir().unwrap();
        let loaded = load_palette(root.path()).unwrap();

        let user = root.path().join(USER_PALETTE);
        assert!(user.exists(), "{} was not seeded", user.display());
        assert_eq!(std::fs::read_to_string(&user).unwrap(), SHIPPED_PALETTE);
        assert_eq!(loaded, shipped());
    }

    #[test]
    fn a_symlinked_palette_path_is_refused_without_touching_its_destination() {
        let root = tempfile::tempdir().unwrap();
        let victim = tempfile::tempdir().unwrap();
        let helm = root.path().join("helm");
        std::os::unix::fs::symlink(victim.path(), &helm).unwrap();

        let err = load_palette(root.path()).expect_err("a symlinked palette directory must be refused");
        assert!(err.to_string().contains("symlink"), "{err}");
        assert!(
            !victim.path().join("palette.toml").exists(),
            "first-run seeding wrote through the symlink"
        );

        std::fs::remove_file(&helm).unwrap();
        std::fs::create_dir(&helm).unwrap();
        let palette_link = helm.join("palette.toml");
        let palette_victim = victim.path().join("palette.toml");
        std::fs::write(&palette_victim, "not a palette\n").unwrap();
        std::os::unix::fs::symlink(&palette_victim, &palette_link).unwrap();

        let err = load_palette(root.path()).expect_err("a symlinked palette file must be refused");
        assert!(err.to_string().contains("symlink"), "{err}");
        assert_eq!(std::fs::read_to_string(palette_victim).unwrap(), "not a palette\n");
    }

    #[test]
    fn lint_report_is_clean_for_the_shipped_palette_and_lists_hue_separations() {
        let report = lint(&shipped());
        assert!(report.is_clean(), "{:?}", report.findings);
        // Derived rather than hardcoded: adding an accent to palette.toml
        // should not require editing this number, only satisfying it.
        let n = shipped().accent.named().len();
        assert_eq!(
            report.separations.len(),
            n * (n - 1) / 2,
            "every accent pair is reported"
        );
        for s in &report.separations {
            assert!(
                s.degrees >= helm_core::palette::MIN_ACCENT_HUE_SEPARATION,
                "{}/{} only {:.1} deg apart",
                s.a,
                s.b,
                s.degrees
            );
        }
    }

    #[test]
    fn diff_reports_what_would_change_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let set = templates();
        apply_with(&shipped(), root.path(), &set, &mut Recorder::default()).unwrap();
        let before = tree(root.path());

        let mut p = shipped();
        p.accent.violet = helm_core::color::Rgb::new(0xb0, 0x7a, 0xff);
        let changes = diff(&p, root.path()).unwrap();

        assert_eq!(changes.len(), set.len());
        assert!(
            changes.iter().any(|c| c.changed),
            "nothing reported changed"
        );
        assert_eq!(before, tree(root.path()), "diff wrote to disk");
    }
}
