//! Pure classification of already completed snapshot reads.

use std::io;

use crate::session::{SessionSnapshotError, SessionSnapshotV1};

/// Outcome of classifying bytes already read from the session snapshot.
#[derive(Debug)]
pub enum SnapshotLoad {
    /// No snapshot exists, so recovery starts from a fresh ledger.
    Fresh,
    /// A closed, semantically valid snapshot is available for recovery.
    Recovered(SessionSnapshotV1),
    /// Snapshot bytes existed but were invalid and must be reported and ignored.
    Rejected(SessionSnapshotError),
}

/// Classify a completed snapshot read without performing any filesystem access.
pub fn classify_snapshot_read(read: io::Result<Vec<u8>>) -> io::Result<SnapshotLoad> {
    match read {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SnapshotLoad::Fresh),
        Err(error) => Err(error),
        Ok(bytes) => match SessionSnapshotV1::from_json(&bytes) {
            Ok(snapshot) => Ok(SnapshotLoad::Recovered(snapshot)),
            Err(error) => Ok(SnapshotLoad::Rejected(error)),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use realm_core::Ledger;

    use super::{classify_snapshot_read, SnapshotLoad};
    use crate::session::SessionSnapshotV1;

    fn valid_snapshot_bytes() -> Vec<u8> {
        SessionSnapshotV1::new(Ledger::new(), Vec::new(), 0)
            .unwrap()
            .to_json()
            .unwrap()
    }

    #[test]
    fn classifies_completed_snapshot_reads_without_file_io() {
        assert!(matches!(
            classify_snapshot_read(Err(io::Error::from(io::ErrorKind::NotFound))).unwrap(),
            SnapshotLoad::Fresh
        ));

        let valid = valid_snapshot_bytes();
        assert!(matches!(
            classify_snapshot_read(Ok(valid.clone())).unwrap(),
            SnapshotLoad::Recovered(_)
        ));

        let malformed = classify_snapshot_read(Ok(b"{".to_vec())).unwrap();
        assert!(matches!(malformed, SnapshotLoad::Rejected(_)));

        let wrong_version = String::from_utf8(valid.clone())
            .unwrap()
            .replacen("\"schema_version\":1", "\"schema_version\":2", 1)
            .into_bytes();
        let rejected = classify_snapshot_read(Ok(wrong_version)).unwrap();
        assert!(matches!(rejected, SnapshotLoad::Rejected(_)));

        let semantic = String::from_utf8(valid)
            .unwrap()
            .replacen("\"active_orbit\":0", "\"active_orbit\":1", 1)
            .into_bytes();
        let SnapshotLoad::Rejected(error) = classify_snapshot_read(Ok(semantic)).unwrap() else {
            panic!("semantic-invalid bytes must be rejected");
        };
        assert!(error.to_string().contains("active orbit"));
    }

    #[test]
    fn non_not_found_read_error_is_fatal() {
        let error = classify_snapshot_read(Err(io::Error::from(io::ErrorKind::PermissionDenied)))
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
