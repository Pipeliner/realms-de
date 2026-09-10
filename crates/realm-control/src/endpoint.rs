use std::path::{Path, PathBuf};

use rustix::net::SocketAddrUnix;

use crate::RealmDir;

/// The exact fixed control-socket descendant and its retained realm capability.
pub struct SocketEndpoint {
    path: PathBuf,
    realm_dir: RealmDir,
    #[allow(dead_code)]
    bind_address: SocketAddrUnix,
}

impl SocketEndpoint {
    pub(crate) fn new(path: PathBuf, realm_dir: RealmDir, bind_address: SocketAddrUnix) -> Self {
        Self {
            path,
            realm_dir,
            bind_address,
        }
    }

    /// Returns the canonical display path for external tools.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Borrows the retained, validated realm directory capability.
    pub fn realm_dir(&self) -> &RealmDir {
        &self.realm_dir
    }
}
