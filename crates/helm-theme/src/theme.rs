//! Palette loading and lint, plus the legacy mutable apply/diff implementation.
//!
//! The public mutable writer items in this module are retained for implementation
//! migration and historical tests. They are not the supported `helm ctl theme
//! apply|diff` contract. Supported apply publishes a sealed generation and
//! supported diff compares against a fully validated current generation as
//! specified by SPEC 0011.

use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::time::{Duration, Instant};

use helm_core::palette::Finding;
use helm_core::Palette;
use rustix::fs::{fchmod, fstat, fsync, mkdirat, openat, FileType, Mode, OFlags};
#[cfg(test)]
use rustix::fs::{renameat, unlinkat, AtFlags, CWD};
use rustix::io::Errno;
use sha2::{Digest, Sha256};

use crate::generation::{
    GenerationPublication, GenerationPublicationOutcome, GenerationReader, GenerationStore,
    OutputDifference,
};
use crate::render::render_derived;
#[cfg(test)]
use crate::Reloader;
use crate::{templates, Error, Reload, Result, Template};

/// The palette helm ships, embedded so a first run has something to copy.
pub const SHIPPED_PALETTE: &str = include_str!("../../../palette.toml");

/// Where the user's palette lives, relative to `$XDG_CONFIG_HOME`.
pub const USER_PALETTE: &str = "helm/palette.toml";

/// The byte-exact built-in launch profile selected by the current apply path.
const BUILT_IN_LAUNCH_PROFILE: &[u8] = b"helm-theme-launch-profile-v1\nnone\n";

/// Suffix of the file each output is staged in before it is renamed into place.
#[cfg(test)]
const STAGING_SUFFIX: &str = ".helm-tmp";

/// Bound stale-name retries without making one collision an availability bug.
#[cfg(test)]
const MAX_STAGING_ATTEMPTS: usize = 64;

/// Makes staging basenames unique within this process.
#[cfg(test)]
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// The opened configuration root and its reporting-only pathname.
#[cfg(test)]
#[derive(Debug)]
struct OutputRoot {
    fd: OwnedFd,
    display: PathBuf,
}

/// A staged output whose remaining operations are relative to its parent.
#[cfg(test)]
#[derive(Debug)]
struct StagedOutput {
    parent_fd: OwnedFd,
    temporary: OsString,
    final_name: OsString,
}

/// Result of the legacy mutable writer.
///
/// This is not the supported apply result. Generation-only apply returns
/// `GenerationPublicationOutcome`; written/unchanged/reloaded compatibility is
/// not promised.
#[derive(Debug)]
#[cfg(test)]
pub struct Applied {
    /// Legacy mutable files whose contents changed and were renamed into place.
    pub written: Vec<PathBuf>,
    /// Legacy mutable files found byte-identical.
    pub unchanged: Vec<PathBuf>,
    /// Legacy reload mechanisms fired by the mutable writer.
    pub reloaded: Vec<&'static str>,
    /// Wall time for the legacy mutable operation.
    pub elapsed: Duration,
}

/// Immutable theme inputs used by generation apply and diff.
///
/// Apply captures this snapshot while holding the exclusive publication lock;
/// diff captures and renders it before taking the shared lock used to validate
/// and compare current. The raw bytes, rather than a serialised [`Palette`],
/// are what a published generation receipt commits to.
#[derive(Debug)]
pub struct ThemeSnapshot {
    palette: Vec<u8>,
    launch_profile: Vec<u8>,
    renderer_options: BTreeMap<String, Vec<u8>>,
    templates: Vec<Template>,
}

impl ThemeSnapshot {
    /// Construct one complete, immutable apply snapshot.
    pub fn new(
        palette: Vec<u8>,
        launch_profile: Vec<u8>,
        renderer_options: BTreeMap<String, Vec<u8>>,
        templates: Vec<Template>,
    ) -> Result<Self> {
        Ok(Self {
            palette,
            launch_profile,
            renderer_options,
            templates,
        })
    }

    fn publication(self) -> Result<GenerationPublication> {
        let palette_text = std::str::from_utf8(&self.palette)
            .map_err(|_| Error::Generation("palette input is not UTF-8".into()))?;
        let palette = Palette::from_toml(palette_text)?;
        let findings = palette.lint();
        if findings.iter().any(|finding| finding.fatal) {
            return Err(Error::PaletteRefused(findings));
        }
        validate_targets(&self.templates)?;

        let catalogue = catalogue_preimage(&self.templates)?;
        let templates = templates_preimage(&self.templates)?;
        let renderer = renderer_preimage(&self.renderer_options)?;
        let derived = palette.derived();
        let mut outputs = Vec::with_capacity(self.templates.len());
        for template in self.templates {
            let target = normalized_generation_target(&template.target)?;
            let rendered = render_derived(&derived, template.id, template.source)?;
            outputs.push((target, rendered.into_bytes()));
        }
        GenerationPublication::new(
            [
                digest_hex(&self.palette),
                digest_hex(&catalogue),
                digest_hex(&templates),
                digest_hex(&renderer),
                digest_hex(&self.launch_profile),
            ],
            outputs,
        )
        .map_err(Error::Generation)
    }
}

/// One result from the test-only legacy mutable-target diff.
#[derive(Debug)]
#[cfg(test)]
struct LegacyChange {
    /// True when rendered bytes differ from that legacy mutable target.
    pub changed: bool,
}

/// A difference between candidate normalized outputs and the fully validated
/// generation selected by `current`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeOutputChange {
    /// The candidate contains this path and current does not.
    Added(PathBuf),
    /// Current contains this path and the candidate does not.
    Removed(PathBuf),
    /// Both contain this path, but their exact bytes differ.
    ByteDifferent(PathBuf),
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
    let root = normalized_root_spelling(root);
    reject_symlinked_palette_path(&root)?;
    let path = root.join(USER_PALETTE);
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        std::fs::write(&path, SHIPPED_PALETTE).map_err(|e| Error::io(&path, e))?;
    }
    Ok(Palette::load(&path)?)
}

/// Read the user palette for `helmctl theme lint` without creating any files.
///
/// The shipped palette is used only when the configuration root, its `helm`
/// directory, or `palette.toml` is absent. Existing entries are opened through
/// retained directory descriptors and must be safe directories or a regular
/// palette file.
pub fn load_lint_palette(root: &Path) -> Result<Palette> {
    let Some(config) = ConfigRoot::open_optional(root)? else {
        return shipped_lint_palette();
    };
    config.load_lint_palette()
}

fn shipped_lint_palette() -> Result<Palette> {
    Palette::from_toml(SHIPPED_PALETTE).map_err(Into::into)
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

/// Capture the built-in theme inputs and activate them as one sealed generation.
///
/// This path never writes mutable template targets or reloads an existing
/// process.  A successful result names the generation selected for future
/// launches; an ambiguous filesystem result is deliberately not converted into
/// an activation claim.
pub fn apply(root: &Path) -> Result<GenerationPublicationOutcome> {
    let config = ConfigRoot::open(root)?;
    let input_root = config
        .fd
        .try_clone()
        .map_err(|error| Error::Generation(error.to_string()))?;
    apply_with_inner(config, || {
        ThemeSnapshot::new(
            read_or_seed_raw_palette(&input_root)?,
            built_in_launch_profile().to_vec(),
            BTreeMap::new(),
            templates(),
        )
    })
}

/// Activate a generation from a snapshot builder.
///
/// The builder runs only after the generation store has acquired its exclusive
/// lock, so every raw input and rendered output belongs to one serialised
/// publication.  It deliberately has no reloader argument.
pub fn apply_with_snapshot<C>(root: &Path, capture: C) -> Result<GenerationPublicationOutcome>
where
    C: FnOnce() -> Result<ThemeSnapshot>,
{
    apply_with_inner(ConfigRoot::open(root)?, capture)
}

fn apply_with_inner<C>(config: ConfigRoot, capture: C) -> Result<GenerationPublicationOutcome>
where
    C: FnOnce() -> Result<ThemeSnapshot>,
{
    let generated = config.open_generated_root()?;
    let store = GenerationStore::open_from_fd(generated).map_err(Error::Generation)?;
    store
        .publish(|| {
            capture()
                .and_then(ThemeSnapshot::publication)
                .map_err(|error| error.to_string())
        })
        .map_err(|error| Error::Generation(error.to_string()))
}

/// Compare the built-in candidate inputs with the fully validated current
/// generation without initializing or mutating generation state.
pub fn diff(root: &Path) -> Result<Vec<ThemeOutputChange>> {
    let config = ConfigRoot::open(root)?;
    let input_root = config
        .fd
        .try_clone()
        .map_err(|error| Error::Generation(error.to_string()))?;
    diff_with_inner(config, || {
        ThemeSnapshot::new(
            read_raw_palette(&input_root)?,
            built_in_launch_profile().to_vec(),
            BTreeMap::new(),
            templates(),
        )
    })
}

/// Compare a captured candidate snapshot with current without creating a
/// generation root, control inode, lease, publication, or recovery mutation.
pub fn diff_with_snapshot<C>(root: &Path, capture: C) -> Result<Vec<ThemeOutputChange>>
where
    C: FnOnce() -> Result<ThemeSnapshot>,
{
    diff_with_inner(ConfigRoot::open(root)?, capture)
}

fn diff_with_inner<C>(config: ConfigRoot, capture: C) -> Result<Vec<ThemeOutputChange>>
where
    C: FnOnce() -> Result<ThemeSnapshot>,
{
    let candidate = capture()?.publication()?;
    let generated = config.open_existing_generated_root()?;
    let reader = GenerationReader::open_from_fd(generated).map_err(Error::Generation)?;
    reader
        .diff_current(candidate.outputs())
        .map(|changes| {
            changes
                .into_iter()
                .map(|(path, difference)| match difference {
                    OutputDifference::Added => ThemeOutputChange::Added(path.into()),
                    OutputDifference::Removed => ThemeOutputChange::Removed(path.into()),
                    OutputDifference::ByteDifferent => {
                        ThemeOutputChange::ByteDifferent(path.into())
                    }
                })
                .collect()
        })
        .map_err(Error::Generation)
}

/// Run the legacy mutable-target writer and reload fan-out.
///
/// This is retained only for focused unit coverage of the descriptor writer.
/// It is not the supported theme-apply boundary: supported [`apply`] captures
/// inputs once, publishes a sealed immutable generation, returns
/// `GenerationPublicationOutcome`, and never reloads on pointer switch. The
/// injected mutable template set and reloader are a test seam, not an
/// alternative supported activation seam.
#[cfg(test)]
fn apply_with(
    palette: &Palette,
    root: &Path,
    templates: &[Template],
    reloader: &mut dyn Reloader,
) -> Result<Applied> {
    legacy_apply_with_inner(palette, root, templates, reloader)
}

#[cfg(test)]
fn legacy_apply_with_inner(
    palette: &Palette,
    root: &Path,
    templates: &[Template],
    reloader: &mut dyn Reloader,
) -> Result<Applied> {
    let started = Instant::now();
    let root = normalized_root_spelling(root);

    validate_targets(templates)?;
    reject_symlinked_targets(&root, templates)?;

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
        rendered.push((t, text));
    }

    let output_root = OutputRoot::open(&root)?;

    // Phase 2: stage the ones that differ.
    let mut unchanged = Vec::new();
    let mut staged: Vec<(&Template, StagedOutput)> = Vec::new();
    for (t, text) in rendered {
        let (parent_fd, final_name) = match output_root.open_parent(&t.target) {
            Ok(destination) => destination,
            Err(e) => {
                discard(&staged)?;
                return Err(e);
            }
        };
        match contents_match(&parent_fd, &final_name, text.as_bytes()) {
            Ok(true) => {
                unchanged.push(output_root.display.join(&t.target));
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                discard(&staged)?;
                return Err(e);
            }
        }
        match stage(parent_fd, final_name, &text) {
            Ok(output) => staged.push((t, output)),
            Err(e) => {
                discard(&staged)?;
                return Err(e);
            }
        }
    }

    // Phase 3: rename, which is atomic per file on the same filesystem.
    let mut written = Vec::with_capacity(staged.len());
    for (i, (template, output)) in staged.iter().enumerate() {
        if let Err(e) = commit(output) {
            discard(&staged[i..])?;
            return Err(e);
        }
        written.push(output_root.display.join(&template.target));
    }

    // Legacy mutable phase 4: one reload per distinct mechanism. This ordering
    // is retained for historical behavior; supported generation apply never
    // reaches this path.
    let mut fired: Vec<&Reload> = Vec::new();
    let mut reloaded = Vec::new();
    for (t, _) in &staged {
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

/// Remove equivalent terminal separators and `.` components so `NOFOLLOW`
/// sees the root's basename.
///
/// An all-separator root (including `/`) and `.` are returned unchanged; this
/// is a lexical spelling adjustment and never resolves any path component.
fn normalized_root_spelling(root: &Path) -> PathBuf {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    let bytes = root.as_os_str().as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes.iter().all(|byte| *byte == b'/') {
        return root.to_path_buf();
    }

    let mut end = bytes.len();
    loop {
        while end > 0 && bytes[end - 1] == b'/' {
            if bytes[..end].iter().all(|byte| *byte == b'/') {
                break;
            }
            end -= 1;
        }

        if end == 1 && bytes[0] == b'.' {
            break;
        }
        let component_start = bytes[..end]
            .iter()
            .rposition(|byte| *byte == b'/')
            .map_or(0, |separator| separator + 1);
        if bytes[component_start..end] != *b"." {
            break;
        }
        end = component_start;
    }

    if end == bytes.len() {
        return root.to_path_buf();
    }
    PathBuf::from(OsString::from_vec(bytes[..end].to_vec()))
}

/// An opened configuration root used to bootstrap the Helm-owned generated
/// subtree without returning to path-based recursive creation.
#[derive(Debug)]
struct ConfigRoot {
    fd: OwnedFd,
}

impl ConfigRoot {
    fn open(root: &Path) -> Result<Self> {
        let root = normalized_root_spelling(root);
        let fd = crate::generation::open_directory_chain(&root).map_err(Error::Generation)?;
        Ok(Self { fd })
    }

    fn open_optional(root: &Path) -> Result<Option<Self>> {
        let root = normalized_root_spelling(root);
        match crate::generation::open_directory_chain_no_follow(&root) {
            Ok(fd) => Ok(Some(Self { fd })),
            Err(Errno::NOENT) => Ok(None),
            Err(error) => Err(Error::Generation(format!("configuration root: {error}"))),
        }
    }

    fn load_lint_palette(&self) -> Result<Palette> {
        let directory_flags =
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let helm = match openat(&self.fd, "helm", directory_flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return shipped_lint_palette(),
            Err(error) => {
                return Err(Error::Generation(format!(
                    "helm palette directory: {error}"
                )))
            }
        };
        let palette = match openat(
            &helm,
            "palette.toml",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return shipped_lint_palette(),
            Err(error) => return Err(Error::Generation(format!("helm palette: {error}"))),
        };
        let raw = read_open_palette(palette)?;
        Ok(Palette::from_toml(std::str::from_utf8(&raw).map_err(
            |error| Error::Generation(format!("helm palette is not UTF-8: {error}")),
        )?)?)
    }

    fn open_generated_root(&self) -> Result<OwnedFd> {
        let helm = self.open_or_create_directory(&self.fd, "helm", "helm", false)?;
        let generated =
            self.open_or_create_directory(&helm, "generated", "helm/generated", true)?;
        Ok(generated)
    }

    fn open_existing_generated_root(&self) -> Result<OwnedFd> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let helm = openat(&self.fd, "helm", flags, Mode::empty())
            .map_err(|error| Error::Generation(format!("helm: {error}")))?;
        let generated = openat(&helm, "generated", flags, Mode::empty())
            .map_err(|error| Error::Generation(format!("helm/generated: {error}")))?;
        let stat = fstat(&generated).map_err(|error| Error::Generation(error.to_string()))?;
        let current_uid = rustix::process::getuid().as_raw();
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
            || stat.st_uid != current_uid
            || Mode::from_raw_mode(stat.st_mode).bits() != 0o700
        {
            return Err(Error::Generation(
                "helm/generated must be a current-UID mode-0700 directory".into(),
            ));
        }
        Ok(generated)
    }

    fn open_or_create_directory(
        &self,
        parent: &OwnedFd,
        name: &str,
        display: &str,
        owned_mode_0700: bool,
    ) -> Result<OwnedFd> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut created = false;
        let fd = match openat(parent, name, flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => {
                match mkdirat(parent, name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                    Ok(()) => created = true,
                    Err(Errno::EXIST) => {}
                    Err(error) => return Err(Error::Generation(format!("{display}: {error}"))),
                }
                openat(parent, name, flags, Mode::empty())
                    .map_err(|error| Error::Generation(format!("{display}: {error}")))?
            }
            Err(error) => return Err(Error::Generation(format!("{display}: {error}"))),
        };
        if created {
            fchmod(&fd, Mode::RUSR | Mode::WUSR | Mode::XUSR)
                .map_err(|error| Error::Generation(format!("{display}: {error}")))?;
        }
        if owned_mode_0700 {
            let stat = fstat(&fd).map_err(|error| Error::Generation(error.to_string()))?;
            let current_uid = rustix::process::getuid().as_raw();
            if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
                || stat.st_uid != current_uid
                || Mode::from_raw_mode(stat.st_mode).bits() != 0o700
            {
                return Err(Error::Generation(
                    "helm/generated must be a current-UID mode-0700 directory".into(),
                ));
            }
        }
        fsync(parent).map_err(|error| Error::Generation(format!("{display}: {error}")))?;
        Ok(fd)
    }
}

fn read_raw_palette(config: &OwnedFd) -> Result<Vec<u8>> {
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let helm = openat(config, "helm", directory_flags, Mode::empty())
        .map_err(|error| Error::Generation(format!("helm palette directory: {error}")))?;
    let palette = openat(
        &helm,
        "palette.toml",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| Error::Generation(format!("helm palette: {error}")))?;
    read_open_palette(palette)
}

fn read_or_seed_raw_palette(config: &OwnedFd) -> Result<Vec<u8>> {
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let helm = openat(config, "helm", directory_flags, Mode::empty())
        .map_err(|error| Error::Generation(format!("helm palette directory: {error}")))?;
    let read_flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let palette = match openat(&helm, "palette.toml", read_flags, Mode::empty()) {
        Ok(palette) => palette,
        Err(Errno::NOENT) => {
            let create_flags =
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            match openat(&helm, "palette.toml", create_flags, Mode::RUSR | Mode::WUSR) {
                Ok(palette) => {
                    let mut palette = std::fs::File::from(palette);
                    palette
                        .write_all(SHIPPED_PALETTE.as_bytes())
                        .map_err(|error| {
                            Error::Generation(format!("helm palette seed: {error}"))
                        })?;
                    fsync(&palette).map_err(|error| {
                        Error::Generation(format!("helm palette seed: {error}"))
                    })?;
                    fsync(&helm).map_err(|error| {
                        Error::Generation(format!("helm palette directory: {error}"))
                    })?;
                    return Ok(SHIPPED_PALETTE.as_bytes().to_vec());
                }
                Err(Errno::EXIST) => openat(&helm, "palette.toml", read_flags, Mode::empty())
                    .map_err(|error| Error::Generation(format!("helm palette: {error}")))?,
                Err(error) => return Err(Error::Generation(format!("helm palette seed: {error}"))),
            }
        }
        Err(error) => return Err(Error::Generation(format!("helm palette: {error}"))),
    };
    read_open_palette(palette)
}

fn read_open_palette(palette: OwnedFd) -> Result<Vec<u8>> {
    let stat = fstat(&palette).map_err(|error| Error::Generation(error.to_string()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(Error::Generation(
            "helm palette must be a regular file".into(),
        ));
    }
    let mut raw = Vec::new();
    std::fs::File::from(palette)
        .read_to_end(&mut raw)
        .map_err(|error| Error::Generation(error.to_string()))?;
    Ok(raw)
}

fn built_in_launch_profile() -> &'static [u8] {
    BUILT_IN_LAUNCH_PROFILE
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn canonical_length_prefixed(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(value.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(value);
}

fn validate_catalogue_text(value: &str, description: &str) -> Result<()> {
    if value.is_empty()
        || !value.bytes().all(|byte| {
            matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'/')
        })
    {
        return Err(Error::Generation(format!(
            "{description} is not manifest-safe"
        )));
    }
    Ok(())
}

fn validate_reload_text(value: &str, description: &str) -> Result<()> {
    if value.contains(['\0', '\r', '\n']) {
        return Err(Error::Generation(format!(
            "{description} contains a forbidden control byte"
        )));
    }
    Ok(())
}

fn normalized_generation_target(target: &Path) -> Result<String> {
    let target = target
        .to_str()
        .ok_or_else(|| Error::Generation("template target is not UTF-8".into()))?;
    if target.is_empty()
        || !target.bytes().all(|byte| {
            matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'/')
        })
    {
        return Err(Error::Generation("template target is not a manifest-safe path".into()));
    }
    Ok(target.into())
}

fn reload_kind(reload: &Reload) -> Result<Vec<u8>> {
    match reload {
        Reload::None => Ok(b"none".to_vec()),
        Reload::HelmClients => Ok(b"helm-clients".to_vec()),
        Reload::Signal { process, signal } => {
            validate_reload_text(process, "reload process")?;
            if *signal < 0 {
                return Err(Error::Generation("reload signal must be unsigned".into()));
            }
            Ok(format!("signal:{}:{process}{signal}", process.len()).into_bytes())
        }
        Reload::Command(argv) => {
            let mut encoded = format!("command:{}:", argv.len()).into_bytes();
            for argument in argv {
                validate_reload_text(argument, "reload command argument")?;
                canonical_length_prefixed(&mut encoded, argument.as_bytes());
            }
            Ok(encoded)
        }
    }
}

fn catalogue_preimage(templates: &[Template]) -> Result<Vec<u8>> {
    let mut templates: Vec<&Template> = templates.iter().collect();
    templates.sort_by(|left, right| left.id.as_bytes().cmp(right.id.as_bytes()));
    if templates.windows(2).any(|pair| pair[0].id == pair[1].id) {
        return Err(Error::Generation("template IDs must be unique".into()));
    }
    let mut encoded = Vec::new();
    for template in templates {
        validate_catalogue_text(template.id, "template ID")?;
        let target = normalized_generation_target(&template.target)?;
        let reload = reload_kind(&template.reload)?;
        canonical_length_prefixed(&mut encoded, template.id.as_bytes());
        canonical_length_prefixed(&mut encoded, target.as_bytes());
        canonical_length_prefixed(&mut encoded, &reload);
        encoded.push(b'\n');
    }
    Ok(encoded)
}

fn templates_preimage(templates: &[Template]) -> Result<Vec<u8>> {
    let mut templates: Vec<&Template> = templates.iter().collect();
    templates.sort_by(|left, right| left.id.as_bytes().cmp(right.id.as_bytes()));
    let mut encoded = Vec::new();
    for template in templates {
        validate_catalogue_text(template.id, "template ID")?;
        canonical_length_prefixed(&mut encoded, template.id.as_bytes());
        canonical_length_prefixed(&mut encoded, template.source.as_bytes());
        encoded.push(b'\n');
    }
    Ok(encoded)
}

fn renderer_preimage(options: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let mut entries: Vec<_> = options.iter().collect();
    entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    let mut encoded = b"helm-theme-renderer-v1\n".to_vec();
    for (name, value) in entries {
        validate_catalogue_text(name, "renderer option name")?;
        canonical_length_prefixed(&mut encoded, name.as_bytes());
        canonical_length_prefixed(&mut encoded, value);
        encoded.push(b'\n');
    }
    Ok(encoded)
}

/// Compare rendered bytes with legacy mutable targets without writing them.
///
/// This is not the supported `theme diff`: it does not validate or compare the
/// generation selected by `current` and must not be presented as satisfying
/// SPEC 0011's read-only generation-aware diff contract.
#[cfg(test)]
fn legacy_diff(palette: &Palette, root: &Path) -> Result<Vec<LegacyChange>> {
    let templates = templates();
    validate_targets(&templates)?;
    let derived = palette.derived();
    let mut out = Vec::new();
    for t in templates {
        let target = root.join(&t.target);
        let text = render_derived(&derived, t.id, t.source)?;
        let changed = !std::fs::read(&target).is_ok_and(|old| old == text.as_bytes());
        out.push(LegacyChange { changed });
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
#[cfg(test)]
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

#[cfg(test)]
impl OutputRoot {
    /// Open the configuration root once so later operations cannot be
    /// redirected by replacing its pathname.
    fn open(root: &Path) -> Result<Self> {
        let fd = openat(
            CWD,
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| Error::io(root, error.into()))?;
        Ok(Self {
            fd,
            display: root.to_path_buf(),
        })
    }

    /// Open or create `target`'s parents without leaving the held descriptor
    /// chain, returning that parent and the normalized final basename.
    fn open_parent(&self, target: &Path) -> Result<(OwnedFd, OsString)> {
        let final_name = target
            .file_name()
            .expect("validate_targets accepted a target without a basename")
            .to_owned();
        let mut parent_fd = self
            .fd
            .try_clone()
            .map_err(|error| Error::io(target, error))?;
        let directory_flags =
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;

        for component in target
            .parent()
            .expect("a normalized target always has a parent")
            .components()
        {
            let Component::Normal(component) = component else {
                unreachable!("validate_targets accepted a non-normal target parent");
            };
            let child_fd = match openat(&parent_fd, component, directory_flags, Mode::empty()) {
                Ok(fd) => fd,
                Err(Errno::NOENT) => {
                    match mkdirat(&parent_fd, component, Mode::RWXU) {
                        Ok(()) | Err(Errno::EXIST) => {}
                        Err(error) => return Err(Error::io(target, error.into())),
                    }
                    openat(&parent_fd, component, directory_flags, Mode::empty())
                        .map_err(|error| Error::io(target, error.into()))?
                }
                Err(error) => return Err(Error::io(target, error.into())),
            };
            parent_fd = child_fd;
        }

        Ok((parent_fd, final_name))
    }
}

/// Compare the final file through its already-opened parent descriptor.
#[cfg(test)]
fn contents_match(parent_fd: &OwnedFd, final_name: &OsStr, expected: &[u8]) -> Result<bool> {
    let fd = match openat(
        parent_fd,
        final_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(false),
        Err(error) => return Err(Error::io(final_name, error.into())),
    };
    let mut file = std::fs::File::from(fd);
    let mut old = Vec::new();
    file.read_to_end(&mut old)
        .map_err(|error| Error::io(final_name, error))?;
    Ok(old == expected)
}

/// Write `text` to a unique sibling through the held parent and flush it.
#[cfg(test)]
fn stage(parent_fd: OwnedFd, final_name: OsString, text: &str) -> Result<StagedOutput> {
    for attempt in 0..MAX_STAGING_ATTEMPTS {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary = OsString::from(".");
        temporary.push(&final_name);
        temporary.push(format!(
            "{STAGING_SUFFIX}.{}.{}",
            std::process::id(),
            sequence
        ));

        let fd = match openat(
            &parent_fd,
            &temporary,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => fd,
            Err(Errno::EXIST) if attempt + 1 < MAX_STAGING_ATTEMPTS => continue,
            Err(error) => return Err(Error::io(&temporary, error.into())),
        };

        let output = StagedOutput {
            parent_fd,
            temporary,
            final_name,
        };
        let mut file = std::fs::File::from(fd);
        let write_result = file
            .write_all(text.as_bytes())
            .and_then(|()| fsync(&file).map_err(std::io::Error::from));
        if let Err(error) = write_result {
            cleanup(&output)?;
            return Err(Error::io(&output.temporary, error));
        }
        return Ok(output);
    }

    unreachable!("the bounded staging loop always returns")
}

/// Atomically publish one staged file and make its directory entry durable.
#[cfg(test)]
fn commit(output: &StagedOutput) -> Result<()> {
    renameat(
        &output.parent_fd,
        &output.temporary,
        &output.parent_fd,
        &output.final_name,
    )
    .map_err(|error| Error::io(&output.final_name, error.into()))?;
    fsync(&output.parent_fd).map_err(|error| Error::io(&output.final_name, error.into()))
}

/// Remove one staging basename through its held parent descriptor.
#[cfg(test)]
fn cleanup(output: &StagedOutput) -> Result<()> {
    match unlinkat(&output.parent_fd, &output.temporary, AtFlags::empty()) {
        Ok(()) | Err(Errno::NOENT) => Ok(()),
        Err(error) => Err(Error::io(&output.temporary, error.into())),
    }
}

/// Remove every still-staged file, ignoring only names already absent.
#[cfg(test)]
fn discard(staged: &[(&Template, StagedOutput)]) -> Result<()> {
    let mut first_error = None;
    for (_, output) in staged {
        if let Err(error) = cleanup(output) {
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::{GenerationPublicationOutcome, GenerationStore};
    use crate::{templates, Reload, Template};
    use std::collections::BTreeMap;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::PathBuf;

    fn shipped() -> Palette {
        Palette::from_toml(SHIPPED_PALETTE).expect("shipped palette must parse")
    }

    fn one_output_snapshot(palette: &Palette) -> Result<ThemeSnapshot> {
        ThemeSnapshot::new(
            palette.to_toml().into_bytes(),
            built_in_launch_profile().to_vec(),
            BTreeMap::new(),
            vec![Template {
                id: "theme",
                source: "accent = {{ accent.violet }}\n",
                target: PathBuf::from("theme.conf"),
                reload: Reload::None,
            }],
        )
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

    #[derive(Debug, PartialEq, Eq)]
    enum InventoryKind {
        Directory,
        Regular(Vec<u8>),
        Symlink(Vec<u8>),
        Other,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct InventoryEntry {
        kind: InventoryKind,
        mode: u32,
        uid: u32,
    }

    /// A no-follow inventory of every path, including directories and control
    /// metadata. This is the write oracle for generation diff: unlike `tree`,
    /// it detects forbidden empty-directory/control initialization.
    fn inventory(root: &Path) -> BTreeMap<PathBuf, InventoryEntry> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(path) = stack.pop() {
            let metadata = std::fs::symlink_metadata(&path).unwrap();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let relative = if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative
            };
            let file_type = metadata.file_type();
            let kind = if file_type.is_dir() {
                let mut children = std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                children.sort();
                stack.extend(children.into_iter().rev());
                InventoryKind::Directory
            } else if file_type.is_file() {
                InventoryKind::Regular(std::fs::read(&path).unwrap())
            } else if file_type.is_symlink() {
                InventoryKind::Symlink(
                    std::fs::read_link(&path)
                        .unwrap()
                        .as_os_str()
                        .as_bytes()
                        .to_vec(),
                )
            } else {
                InventoryKind::Other
            };
            out.insert(
                relative,
                InventoryEntry {
                    kind,
                    mode: metadata.mode() & 0o7777,
                    uid: metadata.uid(),
                },
            );
        }
        out
    }

    #[test]
    fn generation_apply_publishes_changed_bytes_without_writing_mutable_targets_or_a_reloader() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("theme.conf");
        std::fs::write(&target, "mutable target sentinel\n").unwrap();

        let first = apply_with_snapshot(root.path(), || {
            ThemeSnapshot::new(
                SHIPPED_PALETTE.as_bytes().to_vec(),
                built_in_launch_profile().to_vec(),
                BTreeMap::new(),
                vec![Template {
                    id: "theme",
                    source: "accent = {{ accent.violet }}\n",
                    target: PathBuf::from("theme.conf"),
                    reload: Reload::Command(vec!["must-not-run".into()]),
                }],
            )
        })
        .unwrap();

        let mut changed_palette = shipped();
        changed_palette.accent.violet = helm_core::color::Rgb::new(0xb0, 0x7a, 0xff);
        let second = apply_with_snapshot(root.path(), || {
            ThemeSnapshot::new(
                changed_palette.to_toml().into_bytes(),
                built_in_launch_profile().to_vec(),
                BTreeMap::new(),
                vec![Template {
                    id: "theme",
                    source: "accent = {{ accent.violet }}\n",
                    target: PathBuf::from("theme.conf"),
                    reload: Reload::Command(vec!["must-not-run".into()]),
                }],
            )
        })
        .unwrap();

        let GenerationPublicationOutcome::Committed(first_generation) = first else {
            panic!("first generation did not commit cleanly: {first:?}");
        };
        let GenerationPublicationOutcome::Committed(second_generation) = second else {
            panic!("second generation did not commit cleanly: {second:?}");
        };
        assert_ne!(first_generation.as_str(), second_generation.as_str());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "mutable target sentinel\n",
            "generation apply wrote the mutable template target"
        );

        let store = GenerationStore::open(&root.path().join("helm/generated")).unwrap();
        let selected = store.select_current().unwrap();
        assert_eq!(selected.as_str(), second_generation.as_str());
        assert_eq!(
            selected.read_output("theme.conf").unwrap(),
            b"accent = #b07aff\n",
            "the selected generation did not contain the changed rendering"
        );
        selected.release().unwrap();
    }

    #[test]
    fn public_apply_seeds_a_fresh_config_and_publishes_a_valid_generation() {
        let root = tempfile::tempdir().unwrap();
        let palette_path = root.path().join(USER_PALETTE);
        assert!(!palette_path.exists());

        let outcome = apply(root.path()).unwrap();

        assert_eq!(
            std::fs::read(&palette_path).unwrap(),
            SHIPPED_PALETTE.as_bytes()
        );
        let GenerationPublicationOutcome::Committed(generation) = outcome else {
            panic!("first apply did not commit cleanly: {outcome:?}");
        };
        let store = GenerationStore::open(&root.path().join("helm/generated")).unwrap();
        let selected = store.select_current().unwrap();
        assert_eq!(selected.as_str(), generation.as_str());
        let templates = templates();
        assert!(!templates.is_empty());
        for template in templates {
            let target = normalized_generation_target(&template.target).unwrap();
            let bytes = selected.read_output(&target).unwrap();
            assert!(
                !bytes.windows(2).any(|window| window == b"{{"),
                "{} retained an unexpanded placeholder",
                template.id
            );
        }
        selected.release().unwrap();
    }

    #[test]
    fn public_apply_fatal_candidate_preserves_current_generation() {
        let root = tempfile::tempdir().unwrap();
        apply(root.path()).expect("apply initial generation");
        let current = root.path().join("helm/generated/current");
        let before = std::fs::read_to_string(&current).expect("read current generation");
        let palette = root.path().join(USER_PALETTE);
        let invalid = std::fs::read_to_string(&palette)
            .expect("read seeded palette")
            .replace("normal = \"#c2cbde\"", "normal = \"#101218\"");
        std::fs::write(&palette, invalid).expect("write fatally unreadable palette");

        assert!(apply(root.path()).is_err(), "fatal palette was accepted");
        assert_eq!(
            std::fs::read_to_string(&current).expect("read current generation"),
            before,
            "fatal candidate changed the current generation"
        );
    }

    #[test]
    fn public_apply_keeps_an_existing_user_palette_and_publishes_its_derived_output() {
        let root = tempfile::tempdir().unwrap();
        let helm = root.path().join("helm");
        std::fs::create_dir(&helm).unwrap();
        let palette_path = helm.join("palette.toml");
        let mut user_palette = shipped();
        user_palette.accent.violet = helm_core::color::Rgb::new(0xb0, 0x7a, 0xff);
        let user_bytes = user_palette.to_toml();
        std::fs::write(&palette_path, &user_bytes).unwrap();

        let outcome = apply(root.path()).unwrap();

        assert_eq!(std::fs::read_to_string(&palette_path).unwrap(), user_bytes);
        let GenerationPublicationOutcome::Committed(generation) = outcome else {
            panic!("user-palette apply did not commit cleanly: {outcome:?}");
        };
        let store = GenerationStore::open(&root.path().join("helm/generated")).unwrap();
        let selected = store.select_current().unwrap();
        assert_eq!(selected.as_str(), generation.as_str());
        assert!(
            selected
                .read_output("fuzzel/fuzzel.ini")
                .unwrap()
                .windows(b"b07aff".len())
                .any(|window| window == b"b07aff"),
            "the selected generation did not use the existing user palette"
        );
        selected.release().unwrap();
    }

    #[test]
    fn public_apply_refuses_a_fatal_user_palette_without_replacing_current() {
        let root = tempfile::tempdir().unwrap();
        let first = apply(root.path()).unwrap();
        let GenerationPublicationOutcome::Committed(first_generation) = first else {
            panic!("initial apply did not commit cleanly: {first:?}");
        };

        let mut fatal_palette = shipped();
        fatal_palette.text.normal = fatal_palette.background.pane;
        std::fs::write(root.path().join(USER_PALETTE), fatal_palette.to_toml()).unwrap();

        let error = apply(root.path()).expect_err("fatal palette lint must refuse publication");
        assert!(
            error.to_string().contains("text.normal"),
            "fatal palette refusal lost its diagnostic: {error:?}"
        );

        let store = GenerationStore::open(&root.path().join("helm/generated")).unwrap();
        let selected = store.select_current().unwrap();
        assert_eq!(selected.as_str(), first_generation.as_str());
        selected.release().unwrap();
    }

    #[test]
    fn generation_apply_refuses_an_unknown_placeholder_without_replacing_current() {
        let root = tempfile::tempdir().unwrap();
        let first = apply(root.path()).unwrap();
        let GenerationPublicationOutcome::Committed(first_generation) = first else {
            panic!("initial apply did not commit cleanly: {first:?}");
        };

        let error = apply_with_snapshot(root.path(), || {
            ThemeSnapshot::new(
                SHIPPED_PALETTE.as_bytes().to_vec(),
                built_in_launch_profile().to_vec(),
                BTreeMap::new(),
                vec![Template {
                    id: "unknown",
                    source: "value = {{ unknown.path }}\n",
                    target: PathBuf::from("unknown.conf"),
                    reload: Reload::None,
                }],
            )
        })
        .expect_err("unknown placeholders must refuse generation publication");
        assert!(
            error.to_string().contains("unknown.path"),
            "unknown placeholder refusal lost its diagnostic: {error:?}"
        );

        let store = GenerationStore::open(&root.path().join("helm/generated")).unwrap();
        let selected = store.select_current().unwrap();
        assert_eq!(selected.as_str(), first_generation.as_str());
        selected.release().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn public_apply_refuses_a_palette_symlink_without_seeding_its_destination() {
        let root = tempfile::tempdir().unwrap();
        let victim = tempfile::tempdir().unwrap();
        let helm = root.path().join("helm");
        std::fs::create_dir(&helm).unwrap();
        let destination = victim.path().join("palette.toml");
        std::os::unix::fs::symlink(&destination, helm.join("palette.toml")).unwrap();

        let result = apply(root.path());

        assert!(
            result.is_err(),
            "a palette symlink was accepted: {result:?}"
        );
        assert!(
            !destination.exists(),
            "public apply seeded through the palette symlink"
        );
    }

    #[test]
    fn supported_apply_then_diff_is_empty_and_diff_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        apply(root.path()).unwrap();
        let before = inventory(root.path());

        let changes = diff(root.path()).unwrap();

        assert!(
            changes.is_empty(),
            "unchanged output was reported: {changes:?}"
        );
        assert_eq!(
            inventory(root.path()),
            before,
            "generation diff wrote to disk"
        );
    }

    #[test]
    fn generation_diff_reports_only_outputs_affected_by_a_palette_change() {
        let root = tempfile::tempdir().unwrap();
        let templates = || {
            vec![
                Template {
                    id: "teal",
                    source: "accent = {{ accent.teal }}\n",
                    target: PathBuf::from("teal.conf"),
                    reload: Reload::None,
                },
                Template {
                    id: "violet",
                    source: "accent = {{ accent.violet }}\n",
                    target: PathBuf::from("violet.conf"),
                    reload: Reload::None,
                },
            ]
        };
        apply_with_snapshot(root.path(), || {
            ThemeSnapshot::new(
                shipped().to_toml().into_bytes(),
                built_in_launch_profile().to_vec(),
                BTreeMap::new(),
                templates(),
            )
        })
        .unwrap();
        let before = inventory(root.path());
        let mut candidate = shipped();
        candidate.accent.violet = helm_core::color::Rgb::new(0xb0, 0x7a, 0xff);

        let changes = diff_with_snapshot(root.path(), || {
            ThemeSnapshot::new(
                candidate.to_toml().into_bytes(),
                built_in_launch_profile().to_vec(),
                BTreeMap::new(),
                templates(),
            )
        })
        .unwrap();

        assert_eq!(
            changes,
            vec![ThemeOutputChange::ByteDifferent(PathBuf::from(
                "violet.conf"
            ))]
        );
        assert_eq!(
            inventory(root.path()),
            before,
            "generation diff wrote to disk"
        );
    }

    #[test]
    fn generation_diff_reports_sorted_added_removed_and_byte_different_paths() {
        let root = tempfile::tempdir().unwrap();
        apply_with_snapshot(root.path(), || {
            ThemeSnapshot::new(
                shipped().to_toml().into_bytes(),
                built_in_launch_profile().to_vec(),
                BTreeMap::new(),
                vec![
                    Template {
                        id: "changed",
                        source: "old literal\n",
                        target: PathBuf::from("middle.conf"),
                        reload: Reload::None,
                    },
                    Template {
                        id: "removed",
                        source: "removed\n",
                        target: PathBuf::from("z-removed.conf"),
                        reload: Reload::None,
                    },
                ],
            )
        })
        .unwrap();
        let before = inventory(root.path());

        let changes = diff_with_snapshot(root.path(), || {
            ThemeSnapshot::new(
                shipped().to_toml().into_bytes(),
                built_in_launch_profile().to_vec(),
                BTreeMap::new(),
                vec![
                    Template {
                        id: "added",
                        source: "added\n",
                        target: PathBuf::from("a-added.conf"),
                        reload: Reload::None,
                    },
                    Template {
                        id: "changed",
                        source: "new literal\n",
                        target: PathBuf::from("middle.conf"),
                        reload: Reload::None,
                    },
                ],
            )
        })
        .unwrap();

        assert_eq!(
            changes,
            vec![
                ThemeOutputChange::Added(PathBuf::from("a-added.conf")),
                ThemeOutputChange::ByteDifferent(PathBuf::from("middle.conf")),
                ThemeOutputChange::Removed(PathBuf::from("z-removed.conf")),
            ]
        );
        assert_eq!(
            inventory(root.path()),
            before,
            "generation diff wrote to disk"
        );
    }

    #[test]
    fn generation_diff_refuses_clean_absence_without_initializing_state() {
        let root = tempfile::tempdir().unwrap();
        let before = inventory(root.path());

        let result = diff_with_snapshot(root.path(), || one_output_snapshot(&shipped()));

        assert!(result.is_err(), "absent current state was accepted");
        assert_eq!(
            inventory(root.path()),
            before,
            "generation diff initialized state"
        );

        apply_with_snapshot(root.path(), || one_output_snapshot(&shipped())).unwrap();
        std::fs::remove_file(root.path().join("helm/generated/current")).unwrap();
        let before = inventory(root.path());

        let result = diff_with_snapshot(root.path(), || one_output_snapshot(&shipped()));

        assert!(result.is_err(), "absent current pointer was accepted");
        assert_eq!(
            inventory(root.path()),
            before,
            "generation diff mutated clean absent-current state"
        );
    }

    #[test]
    fn generation_diff_refuses_malformed_pending_and_invalid_state_without_writing() {
        for case in ["malformed", "pending", "invalid"] {
            let root = tempfile::tempdir().unwrap();
            let first =
                apply_with_snapshot(root.path(), || one_output_snapshot(&shipped())).unwrap();
            let GenerationPublicationOutcome::Committed(first_generation) = first else {
                panic!("first apply did not commit cleanly: {first:?}");
            };
            let generated = root.path().join("helm/generated");
            match case {
                "malformed" => {
                    std::fs::write(generated.join("current"), b"not-a-generation\n").unwrap();
                }
                "pending" => {
                    let mut changed = shipped();
                    changed.accent.violet = helm_core::color::Rgb::new(0xb0, 0x7a, 0xff);
                    let second =
                        apply_with_snapshot(root.path(), || one_output_snapshot(&changed)).unwrap();
                    let GenerationPublicationOutcome::Committed(second_generation) = second else {
                        panic!("second apply did not commit cleanly: {second:?}");
                    };
                    std::fs::write(
                        generated.join(format!(".current-{}", second_generation.as_str())),
                        format!("{}\n", first_generation.as_str()),
                    )
                    .unwrap();
                }
                "invalid" => {
                    std::fs::write(
                        generated
                            .join("generations")
                            .join(first_generation.as_str())
                            .join("theme.conf"),
                        b"tampered\n",
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let before = inventory(root.path());

            let result = diff_with_snapshot(root.path(), || one_output_snapshot(&shipped()));

            assert!(result.is_err(), "{case} current state was accepted");
            assert_eq!(
                inventory(root.path()),
                before,
                "generation diff mutated {case} state"
            );
        }
    }

    #[test]
    fn generation_diff_refuses_missing_or_unsafe_required_controls_without_mutation() {
        for case in [
            "missing-lock",
            "unsafe-lock-mode",
            "lock-symlink",
            "missing-generations",
            "unsafe-generations-mode",
            "generations-symlink",
            "unsafe-generated-mode",
        ] {
            let root = tempfile::tempdir().unwrap();
            apply_with_snapshot(root.path(), || one_output_snapshot(&shipped())).unwrap();
            let generated = root.path().join("helm/generated");
            let outside = tempfile::tempdir().unwrap();
            match case {
                "missing-lock" => std::fs::remove_file(generated.join("activation.lock")).unwrap(),
                "unsafe-lock-mode" => std::fs::set_permissions(
                    generated.join("activation.lock"),
                    std::fs::Permissions::from_mode(0o644),
                )
                .unwrap(),
                "lock-symlink" => {
                    std::fs::remove_file(generated.join("activation.lock")).unwrap();
                    std::os::unix::fs::symlink(
                        outside.path().join("lock"),
                        generated.join("activation.lock"),
                    )
                    .unwrap();
                }
                "missing-generations" => {
                    std::fs::rename(
                        generated.join("generations"),
                        generated.join("generations-missing"),
                    )
                    .unwrap();
                }
                "unsafe-generations-mode" => std::fs::set_permissions(
                    generated.join("generations"),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap(),
                "generations-symlink" => {
                    std::fs::rename(
                        generated.join("generations"),
                        generated.join("generations-real"),
                    )
                    .unwrap();
                    std::os::unix::fs::symlink("generations-real", generated.join("generations"))
                        .unwrap();
                }
                "unsafe-generated-mode" => {
                    std::fs::set_permissions(&generated, std::fs::Permissions::from_mode(0o755))
                        .unwrap()
                }
                _ => unreachable!(),
            }
            let before = inventory(root.path());

            let result = diff_with_snapshot(root.path(), || one_output_snapshot(&shipped()));

            assert!(result.is_err(), "{case} was accepted");
            assert_eq!(
                inventory(root.path()),
                before,
                "generation diff mutated {case} state"
            );
        }
    }

    #[test]
    fn generation_diff_does_not_require_or_recreate_a_leases_directory() {
        let root = tempfile::tempdir().unwrap();
        apply_with_snapshot(root.path(), || one_output_snapshot(&shipped())).unwrap();
        std::fs::remove_dir(root.path().join("helm/generated/leases")).unwrap();
        let before = inventory(root.path());

        let changes = diff_with_snapshot(root.path(), || one_output_snapshot(&shipped())).unwrap();

        assert!(changes.is_empty());
        assert_eq!(
            inventory(root.path()),
            before,
            "generation diff recreated the unused leases directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn generation_apply_refuses_a_symlinked_configuration_root_ancestor_before_bootstrap() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        let root = parent.path().join("linked-config");
        symlink(destination.path(), &root).unwrap();
        let nested_root = root.join("config");
        std::fs::create_dir(destination.path().join("config")).unwrap();

        let result = apply_with_snapshot(&nested_root, || {
            ThemeSnapshot::new(
                SHIPPED_PALETTE.as_bytes().to_vec(),
                built_in_launch_profile().to_vec(),
                BTreeMap::new(),
                vec![Template {
                    id: "theme",
                    source: "accent = {{ accent.violet }}\n",
                    target: PathBuf::from("theme.conf"),
                    reload: Reload::None,
                }],
            )
        });

        assert!(
            result.is_err(),
            "a root ancestor symlink was accepted: {result:?}"
        );
        assert!(
            !destination.path().join("config/helm/generated").exists(),
            "generation bootstrap wrote through the root ancestor symlink"
        );
    }

    #[test]
    fn generation_apply_rejects_unsafe_catalogue_fields_before_publication() {
        let cases = [
            (
                "unsafe template identifier",
                "unsafe id",
                BTreeMap::new(),
                Reload::None,
            ),
            (
                "unsafe renderer option name",
                "theme",
                BTreeMap::from([("unsafe option".into(), b"value".to_vec())]),
                Reload::None,
            ),
            (
                "negative signal",
                "theme",
                BTreeMap::new(),
                Reload::Signal {
                    process: "theme-client",
                    signal: -1,
                },
            ),
        ];

        for (description, id, renderer_options, reload) in cases {
            let root = tempfile::tempdir().unwrap();
            let result = apply_with_snapshot(root.path(), || {
                ThemeSnapshot::new(
                    SHIPPED_PALETTE.as_bytes().to_vec(),
                    built_in_launch_profile().to_vec(),
                    renderer_options,
                    vec![Template {
                        id,
                        source: "accent = {{ accent.violet }}\n",
                        target: PathBuf::from("theme.conf"),
                        reload,
                    }],
                )
            });
            assert!(result.is_err(), "{description} was accepted: {result:?}");
            assert!(
                !root.path().join("helm/generated/current").exists(),
                "{description} published a generation"
            );
        }
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
    fn a_symlinked_configuration_root_with_a_trailing_separator_is_refused() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let victim = tempfile::tempdir().unwrap();
        let root = parent.path().join("config");
        symlink(victim.path(), &root).unwrap();
        let mut root_with_separator = root.as_os_str().to_owned();
        root_with_separator.push("/");
        let set = [Template {
            id: "root-link-with-separator",
            source: "x = {{ accent.violet }}\n",
            target: PathBuf::from("theme.conf"),
            reload: Reload::None,
        }];

        let applied = apply_with(
            &shipped(),
            Path::new(&root_with_separator),
            &set,
            &mut Recorder::default(),
        );

        assert!(
            applied.is_err(),
            "a trailing separator let apply follow the root symlink: {applied:?}"
        );
        assert!(
            !victim.path().join("theme.conf").exists(),
            "the symlinked configuration root redirected the write"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_configuration_root_with_terminal_dot_is_refused() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let victim = tempfile::tempdir().unwrap();
        let root = parent.path().join("config");
        symlink(victim.path(), &root).unwrap();
        let mut root_with_terminal_dot = root.as_os_str().to_owned();
        root_with_terminal_dot.push("/.");
        let set = [Template {
            id: "root-link-with-terminal-dot",
            source: "x = {{ accent.violet }}\n",
            target: PathBuf::from("theme.conf"),
            reload: Reload::None,
        }];

        let applied = apply_with(
            &shipped(),
            Path::new(&root_with_terminal_dot),
            &set,
            &mut Recorder::default(),
        );

        assert!(
            applied.is_err(),
            "a terminal dot let apply follow the root symlink: {applied:?}"
        );
        assert!(
            !victim.path().join("theme.conf").exists(),
            "the symlinked configuration root redirected the write"
        );
    }

    #[test]
    fn root_normalization_is_lexical_and_preserves_dot_and_slash_roots() {
        for (spelling, normalized) in [
            ("config/.", "config"),
            ("config/.//", "config"),
            ("config/./.", "config"),
            ("config/./child", "config/./child"),
            ("config/..", "config/.."),
            (".", "."),
            ("./", "."),
            ("/", "/"),
            ("///", "///"),
            ("/./", "/"),
        ] {
            assert_eq!(
                normalized_root_spelling(Path::new(spelling)).as_os_str(),
                OsStr::new(normalized),
                "unexpected normalization for {spelling:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_staging_file_is_not_touched() {
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

        let mut palette = shipped();
        palette.contrast = 1.0;
        let applied = apply_with(&palette, root.path(), &set, &mut Recorder::default())
            .expect("a staging symlink must not block a safe unique staging file");

        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "do not overwrite"
        );
        assert!(
            std::fs::symlink_metadata(root.path().join("theme.conf.helm-tmp"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "the planted staging symlink must remain in place"
        );
        assert_eq!(
            std::fs::read_link(root.path().join("theme.conf.helm-tmp")).unwrap(),
            victim
        );
        assert_eq!(applied.written, vec![root.path().join("theme.conf")]);
        assert_eq!(
            std::fs::read_to_string(root.path().join("theme.conf")).unwrap(),
            "x = #a692ec\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_output_parent_cannot_redirect_descriptor_relative_writes() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let live = root.path().join("live");
        std::fs::create_dir(&live).unwrap();
        let output_root = OutputRoot::open(root.path()).unwrap();
        let (commit_parent, final_name) = output_root
            .open_parent(Path::new("live/theme.conf"))
            .unwrap();
        let cleanup_parent = commit_parent.try_clone().unwrap();
        let original_parent = root.path().join("held");
        let victim = tempfile::tempdir().unwrap();

        std::fs::rename(&live, &original_parent).unwrap();
        symlink(victim.path(), &live).unwrap();

        let committed = stage(commit_parent, final_name, "committed\n").unwrap();
        commit(&committed).unwrap();
        let discarded = stage(
            cleanup_parent,
            OsString::from("discarded.conf"),
            "discarded\n",
        )
        .unwrap();
        cleanup(&discarded).unwrap();

        assert!(
            tree(victim.path()).is_empty(),
            "descriptor-owned operations reached the replacement symlink destination"
        );
        assert_eq!(
            std::fs::read_to_string(original_parent.join("theme.conf")).unwrap(),
            "committed\n"
        );
        assert!(!original_parent.join("discarded.conf").exists());
        assert_eq!(
            tree(&original_parent).keys().cloned().collect::<Vec<_>>(),
            vec![PathBuf::from("theme.conf")],
            "staging cleanup left an unexpected entry"
        );
        assert!(
            std::fs::symlink_metadata(&live)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the replacement pathname stopped naming the victim symlink"
        );
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

        let err =
            load_palette(root.path()).expect_err("a symlinked palette directory must be refused");
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
        assert_eq!(
            std::fs::read_to_string(palette_victim).unwrap(),
            "not a palette\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn lint_palette_loader_refuses_every_static_symlinked_root_spelling() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = parent.path().join("config");
        std::fs::create_dir_all(outside.path().join("helm")).unwrap();
        std::fs::write(outside.path().join(USER_PALETTE), "not a palette\n").unwrap();
        symlink(outside.path(), &root).unwrap();

        for suffix in ["", "/", "/.", "/.//"] {
            let mut spelling = root.as_os_str().to_owned();
            spelling.push(suffix);
            let error = load_lint_palette(Path::new(&spelling))
                .expect_err("an existing symlinked root must be refused");
            assert!(
                matches!(error, Error::Generation(_)),
                "unexpected error for {suffix:?}: {error}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn lint_palette_loader_uses_the_retained_root_descriptor_after_path_replacement() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("config");
        let outside = parent.path().join("outside");
        std::fs::create_dir_all(root.join("helm")).unwrap();
        std::fs::create_dir_all(outside.join("helm")).unwrap();
        std::fs::write(
            root.join(USER_PALETTE),
            SHIPPED_PALETTE.replace("name = \"helm-void\"", "name = \"held-root\""),
        )
        .unwrap();
        std::fs::write(outside.join(USER_PALETTE), "not a palette\n").unwrap();

        let config = ConfigRoot::open(&root).expect("open root descriptor before replacement");
        let held_root = parent.path().join("held-root");
        std::fs::rename(&root, &held_root).unwrap();
        symlink(&outside, &root).unwrap();

        let palette = config
            .load_lint_palette()
            .expect("held descriptor must not resolve the replacement pathname");
        assert_eq!(palette.name, "held-root");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_palette_root_with_a_trailing_separator_is_refused_without_initializing_its_destination(
    ) {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let victim = tempfile::tempdir().unwrap();
        let root = parent.path().join("config");
        symlink(victim.path(), &root).unwrap();
        let mut root_with_separator = root.as_os_str().to_owned();
        root_with_separator.push("/");

        let loaded = load_palette(Path::new(&root_with_separator));

        assert!(
            !victim.path().join(USER_PALETTE).exists(),
            "first-run palette initialization wrote through the root symlink"
        );
        assert!(
            loaded.is_err(),
            "a trailing separator let palette initialization follow the root symlink"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_palette_root_with_terminal_dot_is_refused_without_reading_its_destination() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let victim = tempfile::tempdir().unwrap();
        let victim_palette = victim.path().join(USER_PALETTE);
        std::fs::create_dir(victim_palette.parent().unwrap()).unwrap();
        std::fs::write(&victim_palette, SHIPPED_PALETTE).unwrap();
        let before = tree(victim.path());
        let root = parent.path().join("config");
        symlink(victim.path(), &root).unwrap();
        let mut root_with_terminal_dot = root.as_os_str().to_owned();
        root_with_terminal_dot.push("/.");

        let loaded = load_palette(Path::new(&root_with_terminal_dot));

        assert!(
            loaded.is_err(),
            "a terminal dot let palette loading follow the root symlink"
        );
        assert_eq!(
            tree(victim.path()),
            before,
            "palette loading mutated the root symlink destination"
        );
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
    fn legacy_diff_reports_what_would_change_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let set = templates();
        apply_with(&shipped(), root.path(), &set, &mut Recorder::default()).unwrap();
        let before = tree(root.path());

        let mut p = shipped();
        p.accent.violet = helm_core::color::Rgb::new(0xb0, 0x7a, 0xff);
        let changes = legacy_diff(&p, root.path()).unwrap();

        assert_eq!(changes.len(), set.len());
        assert!(
            changes.iter().any(|c| c.changed),
            "nothing reported changed"
        );
        assert_eq!(before, tree(root.path()), "diff wrote to disk");
    }
}
