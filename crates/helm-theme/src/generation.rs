//! Immutable generation manifest validation.

use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::{Component, Path};

use rustix::fs::{openat, Mode, OFlags, CWD};

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
        if value.is_empty()
            || !value.is_ascii()
            || value
                .bytes()
                .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
        {
            return Err("generation id must be non-empty safe ASCII".into());
        }
        Ok(Self(value.into()))
    }
}

/// One validated canonical manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    entries: Vec<(String, String)>,
}

impl Manifest {
    /// Parse `path digest` lines sorted by path.
    pub fn parse(input: &str) -> std::result::Result<Self, String> {
        let mut entries = Vec::new();
        let mut previous = None;
        for line in input.lines() {
            let (path, digest) = line.split_once(' ').ok_or("manifest line lacks digest")?;
            if path.is_empty() || digest.is_empty() || !safe_relative(path) {
                return Err("unsafe manifest path".into());
            }
            if !sha256_digest(digest) {
                return Err("manifest digest must be lowercase SHA-256 hex".into());
            }
            if previous.is_some_and(|last: &str| last >= path) {
                return Err("manifest paths must be unique and sorted".into());
            }
            previous = Some(path);
            entries.push((path.into(), digest.into()));
        }
        Ok(Self { entries })
    }

    /// Verify every manifest-listed regular file against its SHA-256 digest.
    pub fn verify(&self, root: &Path) -> std::result::Result<(), String> {
        let root = GenerationRoot::open(root)?;
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

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains("//")
        && !path.starts_with("./")
        && !path.ends_with('/')
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
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

    #[test]
    fn generation_id_rejects_paths_and_non_ascii() {
        assert!(GenerationId::parse("../old").is_err());
        assert!(GenerationId::parse("génération").is_err());
    }

    #[test]
    fn manifest_rejects_unsorted_duplicate_and_traversal_paths() {
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
    fn manifest_verification_rejects_changed_file_bytes() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("theme.ini"), "new bytes").unwrap();
        let manifest = Manifest::parse(
            "theme.ini 25e4c9b9dd1bb85ba6c0664b75f8c85be4bffc498fab284b95fb5b18cc87e7d5\n",
        )
        .unwrap();
        assert!(manifest.verify(directory.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verification_rejects_symlinked_file() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("actual"), "bytes").unwrap();
        symlink("actual", directory.path().join("theme.ini")).unwrap();
        let manifest = Manifest::parse(
            "theme.ini 277089d91c0bdf4f2e6862ba7e4a07605119431f5d13f726dd352b06f1b206a9\n",
        )
        .unwrap();
        assert!(manifest.verify(directory.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verification_rejects_symlinked_parent() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("outside")).unwrap();
        std::fs::write(directory.path().join("outside/theme.ini"), "bytes").unwrap();
        symlink("outside", directory.path().join("themes")).unwrap();
        let manifest = Manifest::parse(
            "themes/theme.ini 277089d91c0bdf4f2e6862ba7e4a07605119431f5d13f726dd352b06f1b206a9\n",
        )
        .unwrap();
        assert!(manifest.verify(directory.path()).is_err());
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
        let manifest = Manifest::parse(
            "theme.ini 277089d91c0bdf4f2e6862ba7e4a07605119431f5d13f726dd352b06f1b206a9\n",
        )
        .unwrap();

        assert!(manifest
            .verify(&base.path().join("link/generation"))
            .is_err());
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
