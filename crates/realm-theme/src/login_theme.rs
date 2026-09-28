//! Immutable theme selection handed from graphical login to session clients.
use crate::generation::{GenerationId, GenerationSelection, GenerationStore};
use rustix::fs::{flock, FlockOperation};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    config_root: PathBuf,
    generation: String,
    owner_pid: u32,
    start_time: u64,
    boot_id: String,
}

fn boot_id() -> Result<String, String> {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|value| value.trim().to_owned())
        .map_err(|error| error.to_string())
}

fn identity(pid: u32) -> Result<Option<u64>, String> {
    let path = PathBuf::from(format!("/proc/{pid}"));
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.uid() != rustix::process::getuid().as_raw() {
        return Err("login owner belongs to another user".into());
    }
    let stat = match fs::read_to_string(path.join("stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .ok_or("invalid process stat")?
        .1
        .split_whitespace()
        .collect();
    if matches!(fields.first(), Some(&"Z" | &"X")) {
        return Ok(None);
    }
    fields
        .get(19)
        .ok_or("missing process start time")?
        .parse::<u64>()
        .map(Some)
        .map_err(|error| error.to_string())
}

impl Record {
    fn validate(&self) -> Result<(), String> {
        GenerationId::parse(&self.generation)?;
        let valid_boot = self.boot_id.len() == 36
            && self.boot_id.bytes().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            });
        if !self.config_root.is_absolute()
            || self.owner_pid == 0
            || self.start_time == 0
            || !valid_boot
        {
            return Err("malformed login theme record".into());
        }
        Ok(())
    }

    fn live(&self) -> Result<bool, String> {
        Ok(self.boot_id == boot_id()? && identity(self.owner_pid)? == Some(self.start_time))
    }
}

fn read_record(path: &Path) -> Result<Record, String> {
    let record: Record = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("login theme record: {error}"))?,
    )
    .map_err(|error| format!("malformed login theme record: {error}"))?;
    record.validate()?;
    Ok(record)
}

/// Prepare the graphical entry's selection.
pub fn prepare(config_root: &Path, runtime_dir: &Path, owner_pid: u32) -> Result<(), String> {
    if !config_root.is_absolute() || !runtime_dir.is_absolute() {
        return Err("login theme configuration and runtime roots must be absolute".into());
    }
    let start_time = identity(owner_pid)?.ok_or("login owner is not live")?;
    let directory = runtime_dir.join("realm");
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.to_string()),
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(directory.join("session-theme.lock"))
        .map_err(|error| error.to_string())?;
    flock(&lock, FlockOperation::LockExclusive).map_err(|error| error.to_string())?;
    let path = directory.join("session-theme.json");
    match fs::symlink_metadata(&path) {
        Ok(_) if read_record(&path)?.live()? => {
            return Err("a Realm login theme owner is already live".into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    crate::ensure_current(config_root).map_err(|error| error.to_string())?;
    let config_root = fs::canonicalize(config_root).map_err(|error| error.to_string())?;
    let store = GenerationStore::open(&config_root.join("realm/generated"))?;
    let selection = store.select_current_for_process(owner_pid)?;
    selection.read_output("realm/palette.toml").map_err(|error| {
        format!("selected theme lacks a valid palette snapshot: {error}; run realmctl theme apply before login")
    })?;
    let record = Record {
        config_root,
        generation: selection.as_str().into(),
        owner_pid,
        start_time,
        boot_id: boot_id()?,
    };
    if !record.live()? {
        return Err("login owner exited during theme preparation".into());
    }
    let bytes = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
    let temporary = directory.join(format!(".session-theme-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    let published = file
        .write_all(&bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary, &path));
    if let Err(error) = published {
        let _ = fs::remove_file(&temporary);
        return Err(format!("publish login theme record: {error}"));
    }
    selection.retain_process_lease();
    Ok(())
}
/// Load the selection belonging to the live graphical entry.
pub fn load(runtime_dir: &Path) -> Result<GenerationSelection, String> {
    if !runtime_dir.is_absolute() {
        return Err("login runtime root must be absolute".into());
    }
    let record = read_record(&runtime_dir.join("realm/session-theme.json"))?;
    if !record.live()? {
        return Err("login theme owner is no longer live".into());
    }
    let store = GenerationStore::open(&record.config_root.join("realm/generated"))?;
    store.select_generation(&GenerationId::parse(&record.generation)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn login_rejects_old_generation_without_palette_snapshot_before_publication() {
        let root = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        crate::apply(root.path()).unwrap();
        let store = GenerationStore::open(&root.path().join("realm/generated")).unwrap();
        store.publish(legacy_generation).unwrap();
        let current = fs::read(root.path().join("realm/generated/current")).unwrap();
        let error = prepare(root.path(), runtime.path(), std::process::id()).unwrap_err();
        assert!(error.contains("realmctl theme apply"), "{error}");
        assert!(!runtime.path().join("realm/session-theme.json").exists());
        assert_eq!(
            fs::read(root.path().join("realm/generated/current")).unwrap(),
            current
        );
        let repaired = crate::apply(root.path()).unwrap();
        prepare(root.path(), runtime.path(), std::process::id()).unwrap();
        let selected = load(runtime.path()).unwrap();
        assert_eq!(selected.as_str(), repaired.as_str());
        assert_eq!(
            selected.read_output("realm/palette.toml").unwrap(),
            crate::SHIPPED_PALETTE.as_bytes()
        );
    }

    fn legacy_generation() -> Result<crate::generation::GenerationPublication, String> {
        crate::generation::GenerationPublication::new(
            std::array::from_fn(|_| "a".repeat(64)),
            vec![("foot/foot.ini".into(), b"legacy output\n".to_vec())],
        )
    }

    #[test]
    fn login_record_survives_current_removal_and_rejects_competing_login() {
        let root = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        prepare(root.path(), runtime.path(), std::process::id()).unwrap();
        let first = load(runtime.path()).unwrap();
        assert_eq!(
            std::fs::metadata(runtime.path().join("realm"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let record = runtime.path().join("realm/session-theme.json");
        assert_eq!(
            std::fs::metadata(&record).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let original = std::fs::read(&record).unwrap();
        assert!(prepare(root.path(), runtime.path(), std::process::id()).is_err());
        assert_eq!(std::fs::read(&record).unwrap(), original);
        std::fs::remove_file(root.path().join("realm/generated/current")).unwrap();
        assert_eq!(load(runtime.path()).unwrap().as_str(), first.as_str());
    }

    #[test]
    fn malformed_and_stale_records_never_fall_back() {
        let root = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        assert!(load(runtime.path()).is_err());
        assert!(prepare(Path::new("relative"), runtime.path(), std::process::id()).is_err());
        prepare(root.path(), runtime.path(), std::process::id()).unwrap();
        let path = runtime.path().join("realm/session-theme.json");
        let mut record: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record["start_time"] = serde_json::json!(1);
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(load(runtime.path()).is_err());
        prepare(root.path(), runtime.path(), std::process::id()).unwrap();
        std::fs::write(&path, b"broken").unwrap();
        assert!(load(runtime.path()).is_err());
        assert!(prepare(root.path(), runtime.path(), std::process::id()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    }
}
