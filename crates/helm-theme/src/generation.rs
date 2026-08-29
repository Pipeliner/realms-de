//! Immutable generation manifest validation.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::{Component, Path};

use rustix::fs::{openat, Dir, Mode, OFlags, CWD};

/// The held generation root used for every descriptor-relative read.
#[derive(Debug)]
struct GenerationRoot {
    fd: OwnedFd,
}

impl GenerationRoot {
    fn open(root: &Path) -> std::result::Result<Self, String> {
        let fd = open_directory_chain(root)?;
        Ok(Self { fd })
    }

    fn open_parent(&self, target: &Path) -> std::result::Result<(OwnedFd, OsString), String> {
        let final_name = target
            .file_name()
            .ok_or("manifest path lacks a filename")?
            .to_owned();
        let mut parent_fd = self.fd.try_clone().map_err(|error| error.to_string())?;
        let directory_flags =
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;

        for component in target
            .parent()
            .ok_or("manifest path lacks a parent")?
            .components()
        {
            let Component::Normal(component) = component else {
                return Err("unsafe manifest path".into());
            };
            parent_fd = openat(&parent_fd, component, directory_flags, Mode::empty())
                .map_err(|error| error.to_string())?;
        }
        Ok((parent_fd, final_name))
    }
}

/// Open a directory pathname a component at a time, never following a symlink.
fn open_directory_chain(path: &Path) -> std::result::Result<OwnedFd, String> {
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut fd = match path.components().next() {
        Some(Component::RootDir) => openat(CWD, "/", directory_flags, Mode::empty()),
        Some(Component::Prefix(_)) => return Err("unsupported generation root prefix".into()),
        _ => openat(CWD, ".", directory_flags, Mode::empty()),
    }
    .map_err(|error| error.to_string())?;

    for component in path.components() {
        let Component::Normal(component) = component else {
            if matches!(component, Component::RootDir | Component::CurDir) {
                continue;
            }
            return Err("generation root must not contain parent traversal".into());
        };
        fd = openat(&fd, component, directory_flags, Mode::empty())
            .map_err(|error| error.to_string())?;
    }
    Ok(fd)
}

/// Open one manifest file without following any component below `root`.
fn open_regular_file(
    root: &GenerationRoot,
    target: &Path,
) -> std::result::Result<std::fs::File, String> {
    let (parent_fd, final_name) = root.open_parent(target)?;
    let fd = openat(
        &parent_fd,
        &final_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let file = std::fs::File::from(fd);
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("manifest target is not a regular file".into());
    }
    Ok(file)
}

/// An opaque, filesystem-safe generation identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationId(String);

impl GenerationId {
    /// Parse the stable ASCII identifier stored in `current`.
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("generation id must be 128-bit lowercase hex".into());
        }
        Ok(Self(value.into()))
    }
}

/// One validated canonical manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    entries: Vec<(String, String)>,
    v1: V1Manifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct V1Manifest {
    generation: GenerationId,
    input_digests: [String; 5],
    receipt_digest: String,
    raw: String,
}

impl Manifest {
    /// Parse the canonical v1 manifest encoding.
    pub fn parse(input: &str) -> std::result::Result<Self, String> {
        Self::parse_v1(input)
    }

    fn parse_v1(input: &str) -> std::result::Result<Self, String> {
        if input.contains('\r') || !input.ends_with('\n') {
            return Err("v1 manifest must use LF-terminated lines".into());
        }
        let mut lines = input[..input.len() - 1].split('\n');
        if lines.next() != Some("helm-generation-manifest-v1") {
            return Err("unsupported manifest version".into());
        }
        let generation = GenerationId::parse(record_value(&mut lines, "generation")?)?;
        let field_names = [
            "palette-sha256",
            "catalogue-sha256",
            "templates-sha256",
            "renderer-sha256",
            "launch-profile-sha256",
        ];
        let mut input_digests = Vec::new();
        for field in field_names {
            let digest = record_value(&mut lines, field)?;
            if !sha256_digest(digest) {
                return Err("manifest digest must be lowercase SHA-256 hex".into());
            }
            input_digests.push(digest.into());
        }
        let receipt_digest = record_value(&mut lines, "receipt-sha256")?;
        if !sha256_digest(receipt_digest) {
            return Err("manifest digest must be lowercase SHA-256 hex".into());
        }
        let mut entries = Vec::new();
        let mut previous = None;
        for line in lines {
            let Some(rest) = line.strip_prefix("output ") else {
                return Err("unexpected manifest record".into());
            };
            let (path, digest) = rest.split_once(' ').ok_or("output record lacks digest")?;
            if path.is_empty() || !safe_output_path(path) || !sha256_digest(digest) {
                return Err("unsafe manifest output record".into());
            }
            if previous.is_some_and(|last: &str| last >= path) {
                return Err("manifest paths must be unique and sorted".into());
            }
            previous = Some(path);
            entries.push((path.into(), digest.into()));
        }
        if entries.is_empty() {
            return Err("v1 manifest requires an output record".into());
        }
        let input_digests: [String; 5] = input_digests
            .try_into()
            .map_err(|_| "v1 manifest requires five input digests")?;
        Ok(Self {
            entries,
            v1: V1Manifest {
                generation,
                input_digests,
                receipt_digest: receipt_digest.into(),
                raw: input.into(),
            },
        })
    }

    /// Verify every manifest-listed regular file against its SHA-256 digest.
    pub fn verify(&self, root_path: &Path) -> std::result::Result<(), String> {
        let root = GenerationRoot::open(root_path)?;
        verify_v1_metadata(&root, root_path, &self.v1)?;
        validate_v1_tree(&root, &self.entries)?;
        for (path, expected) in &self.entries {
            let mut file = open_regular_file(&root, Path::new(path))?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            let actual = format!("{:x}", Sha256::digest(bytes));
            if &actual != expected {
                return Err("manifest digest mismatch".into());
            }
        }
        Ok(())
    }
}

fn validate_v1_tree(
    root: &GenerationRoot,
    entries: &[(String, String)],
) -> std::result::Result<(), String> {
    let mut files = BTreeSet::from([
        String::from("manifest"),
        String::from("receipt"),
        String::from("seal"),
    ]);
    let mut directories = BTreeSet::new();
    for (path, _) in entries {
        files.insert(path.clone());
        let mut prefix = String::new();
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                break;
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            directories.insert(prefix.clone());
        }
    }
    validate_directory_entries(&root.fd, "", &files, &directories)
}

fn validate_directory_entries(
    fd: &OwnedFd,
    prefix: &str,
    files: &BTreeSet<String>,
    directories: &BTreeSet<String>,
) -> std::result::Result<(), String> {
    let mut directory = Dir::read_from(fd).map_err(|error| error.to_string())?;
    while let Some(entry) = directory.read() {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_bytes();
        if matches!(name, b"." | b"..") {
            continue;
        }
        let name = std::str::from_utf8(name).map_err(|_| "non-UTF-8 tree entry")?;
        let path = if prefix.is_empty() {
            name.into()
        } else {
            format!("{prefix}/{name}")
        };
        if files.contains(&path) {
            let child = openat(
                fd,
                name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
            if !std::fs::File::from(child)
                .metadata()
                .map_err(|error| error.to_string())?
                .is_file()
            {
                return Err("sealed tree contains a non-regular listed file".into());
            }
        } else if directories.contains(&path) {
            let child = openat(
                fd,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
            validate_directory_entries(&child, &path, files, directories)?;
        } else {
            return Err("sealed tree contains an unlisted entry".into());
        }
    }
    Ok(())
}

fn verify_v1_metadata(
    root: &GenerationRoot,
    root_path: &Path,
    v1: &V1Manifest,
) -> std::result::Result<(), String> {
    if root_path.file_name().and_then(|name| name.to_str()) != Some(&v1.generation.0) {
        return Err("generation directory does not match manifest identity".into());
    }
    let manifest = read_regular_bytes(root, Path::new("manifest"))?;
    if manifest != v1.raw.as_bytes() {
        return Err("manifest bytes changed after parsing".into());
    }
    let receipt = read_regular_bytes(root, Path::new("receipt"))?;
    if digest_hex(&receipt) != v1.receipt_digest {
        return Err("receipt digest mismatch".into());
    }
    let expected_receipt = format!(
        "helm-generation-receipt-v1\ngeneration {}\npalette-sha256 {}\ncatalogue-sha256 {}\ntemplates-sha256 {}\nrenderer-sha256 {}\nlaunch-profile-sha256 {}\n",
        v1.generation.0,
        v1.input_digests[0],
        v1.input_digests[1],
        v1.input_digests[2],
        v1.input_digests[3],
        v1.input_digests[4],
    );
    if receipt != expected_receipt.as_bytes() {
        return Err("receipt does not match manifest inputs".into());
    }
    let seal = read_regular_bytes(root, Path::new("seal"))?;
    if seal != format!("{}\n", digest_hex(v1.raw.as_bytes())).as_bytes() {
        return Err("manifest seal mismatch".into());
    }
    Ok(())
}

fn read_regular_bytes(
    root: &GenerationRoot,
    target: &Path,
) -> std::result::Result<Vec<u8>, String> {
    let mut file = open_regular_file(root, target)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn record_value<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    name: &str,
) -> std::result::Result<&'a str, String> {
    let line = lines
        .next()
        .ok_or("manifest ended before required record")?;
    line.strip_prefix(&format!("{name} "))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("expected {name} record"))
}

fn safe_output_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .bytes()
            .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'/'))
        && !path.starts_with('/')
        && !path.ends_with('/')
        && !path.contains("//")
        && path.split('/').all(|component| {
            !matches!(component, "." | ".." | "manifest" | "receipt" | "seal" | "activation.lock")
                && !component.starts_with(".staging-")
        })
}

fn sha256_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn v1_manifest_with_output(path: &str) -> String {
        let generation_id = "0123456789abcdef0123456789abcdef";
        let digest = "0".repeat(64);
        format!(
            "helm-generation-manifest-v1\ngeneration {generation_id}\npalette-sha256 {digest}\ncatalogue-sha256 {digest}\ntemplates-sha256 {digest}\nrenderer-sha256 {digest}\nlaunch-profile-sha256 {digest}\nreceipt-sha256 {digest}\noutput {path} {digest}\n"
        )
    }

    fn v1_fixture(base: &Path, output_path: &str, output: &[u8]) -> (std::path::PathBuf, Manifest) {
        let generation_id = "0123456789abcdef0123456789abcdef";
        let generation = base.join(generation_id);
        std::fs::create_dir(&generation).unwrap();
        let input_digest = digest(b"input");
        let receipt = format!(
            "helm-generation-receipt-v1\ngeneration {generation_id}\npalette-sha256 {input_digest}\ncatalogue-sha256 {input_digest}\ntemplates-sha256 {input_digest}\nrenderer-sha256 {input_digest}\nlaunch-profile-sha256 {input_digest}\n"
        );
        let output_file = generation.join(output_path);
        std::fs::create_dir_all(output_file.parent().unwrap()).unwrap();
        std::fs::write(&output_file, output).unwrap();
        std::fs::write(generation.join("receipt"), &receipt).unwrap();
        let manifest = format!(
            "helm-generation-manifest-v1\ngeneration {generation_id}\npalette-sha256 {input_digest}\ncatalogue-sha256 {input_digest}\ntemplates-sha256 {input_digest}\nrenderer-sha256 {input_digest}\nlaunch-profile-sha256 {input_digest}\nreceipt-sha256 {}\noutput {output_path} {}\n",
            digest(receipt.as_bytes()),
            digest(output),
        );
        std::fs::write(generation.join("manifest"), &manifest).unwrap();
        std::fs::write(
            generation.join("seal"),
            format!("{}\n", digest(manifest.as_bytes())),
        )
        .unwrap();
        (generation, Manifest::parse(&manifest).unwrap())
    }

    #[test]
    fn v1_manifest_rejects_noncanonical_and_reserved_output_paths() {
        for path in [
            "café.ini",
            "theme file",
            "a/./b",
            "manifest",
            "receipt",
            "seal",
            "activation.lock",
            ".staging-0123456789abcdef0123456789abcdef",
        ] {
            assert!(
                Manifest::parse(&v1_manifest_with_output(path)).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn v1_manifest_verifies_its_receipt_seal_and_output() {
        let base = tempfile::tempdir().unwrap();
        let (generation, manifest) = v1_fixture(base.path(), "themes/theme.ini", b"sealed output");
        manifest.verify(&generation).unwrap();
    }

    #[test]
    fn generation_id_rejects_paths_and_non_ascii() {
        assert!(GenerationId::parse("../old").is_err());
        assert!(GenerationId::parse("génération").is_err());
    }

    #[test]
    fn manifest_rejects_legacy_and_malformed_records() {
        assert!(Manifest::parse("b 00\na 00\n").is_err());
        assert!(Manifest::parse("a 00\na 00\n").is_err());
        assert!(Manifest::parse("../a 00\n").is_err());
        assert!(Manifest::parse("a//b 00\n").is_err());
        assert!(Manifest::parse("a not-a-sha256\n").is_err());
        assert!(Manifest::parse(
            "a 277089D91C0BDF4F2E6862BA7E4A07605119431F5D13F726DD352B06F1B206A9\n"
        )
        .is_err());
    }

    #[test]
    fn manifest_verification_rejects_changed_output_bytes() {
        let base = tempfile::tempdir().unwrap();
        let (generation, manifest) = v1_fixture(base.path(), "theme.ini", b"sealed output");
        std::fs::write(generation.join("theme.ini"), "new bytes").unwrap();
        assert!(manifest.verify(&generation).is_err());
    }

    #[test]
    fn v1_verifier_rejects_unlisted_file_and_empty_directory() {
        let file_base = tempfile::tempdir().unwrap();
        let (file_generation, file_manifest) =
            v1_fixture(file_base.path(), "theme.ini", b"sealed output");
        std::fs::write(file_generation.join("unexpected"), "not listed").unwrap();
        assert!(file_manifest.verify(&file_generation).is_err());

        let directory_base = tempfile::tempdir().unwrap();
        let (directory_generation, directory_manifest) =
            v1_fixture(directory_base.path(), "theme.ini", b"sealed output");
        std::fs::create_dir(directory_generation.join("empty")).unwrap();
        assert!(directory_manifest.verify(&directory_generation).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verification_rejects_symlinked_file() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("actual"), "bytes").unwrap();
        symlink("actual", directory.path().join("theme.ini")).unwrap();
        let root = GenerationRoot::open(directory.path()).unwrap();
        assert!(open_regular_file(&root, Path::new("theme.ini")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verification_rejects_symlinked_parent() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("outside")).unwrap();
        std::fs::write(directory.path().join("outside/theme.ini"), "bytes").unwrap();
        symlink("outside", directory.path().join("themes")).unwrap();
        let root = GenerationRoot::open(directory.path()).unwrap();
        assert!(open_regular_file(&root, Path::new("themes/theme.ini")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verification_rejects_a_symlinked_root_ancestor() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("real");
        let generation = real.join("generation");
        std::fs::create_dir_all(&generation).unwrap();
        std::fs::write(generation.join("theme.ini"), "bytes").unwrap();
        symlink(&real, base.path().join("link")).unwrap();
        assert!(GenerationRoot::open(&base.path().join("link/generation")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn held_descriptor_read_cannot_be_redirected_by_replacing_a_parent() {
        use std::io::Read;
        use std::os::unix::fs::symlink;

        let generation = tempfile::tempdir().unwrap();
        let themes = generation.path().join("themes");
        std::fs::create_dir(&themes).unwrap();
        std::fs::write(themes.join("theme.ini"), "sealed bytes").unwrap();
        let root = GenerationRoot::open(generation.path()).unwrap();
        let mut held = open_regular_file(&root, Path::new("themes/theme.ini")).unwrap();
        let moved = generation.path().join("held");
        let victim = tempfile::tempdir().unwrap();

        std::fs::rename(&themes, &moved).unwrap();
        symlink(victim.path(), &themes).unwrap();

        let mut bytes = String::new();
        held.read_to_string(&mut bytes).unwrap();
        assert_eq!(bytes, "sealed bytes");
        assert!(victim.path().read_dir().unwrap().next().is_none());
    }
}
