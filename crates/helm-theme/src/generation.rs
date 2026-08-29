//! Immutable generation manifest validation.

use sha2::{Digest, Sha256};
use std::path::{Component, Path};

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
        for (path, expected) in &self.entries {
            let candidate = root.join(path);
            let metadata = std::fs::symlink_metadata(&candidate).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err("manifest target is not a regular file".into());
            }
            let bytes = std::fs::read(&candidate).map_err(|e| e.to_string())?;
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
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
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
}
