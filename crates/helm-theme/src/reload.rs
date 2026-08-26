//! Telling live programs the theme changed.

use crate::{Reload, Result};

/// Executes reload mechanisms.
pub trait Reloader {
    /// Fire one mechanism. Called at most once per distinct mechanism.
    fn reload(&mut self, reload: &Reload) -> Result<()>;
}

impl<F: FnMut(&Reload) -> Result<()>> Reloader for F {
    fn reload(&mut self, reload: &Reload) -> Result<()> {
        self(reload)
    }
}

/// The real fan-out: signals real processes and runs real commands.
pub struct SystemReloader;

impl Reloader for SystemReloader {
    fn reload(&mut self, reload: &Reload) -> Result<()> {
        let _ = reload;
        unimplemented!()
    }
}
