//! Telling live programs the theme changed.
//!
//! The fan-out runs once, after every file is in place, and once per distinct
//! mechanism rather than once per file. Both halves of that matter: reloading
//! early shows a half-applied theme, and reloading per file makes GTK rebuild
//! its style cascade twice for one apply.

use rustix::process::{Pid, Signal};

use crate::{Error, Reload, Result};

/// Executes reload mechanisms.
///
/// A trait rather than a hardcoded call so a test can count the fan-out without
/// a desktop underneath it — and so M2's control socket can be injected here
/// rather than reached for from inside this crate.
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
        match reload {
            Reload::None => Ok(()),
            Reload::Signal { process, signal } => signal_all(process, *signal),
            Reload::Command(argv) => run(argv),
            // The control socket arrives with helm-session in M2. Nothing in
            // the M1 template set uses this, and saying so is better than a
            // silent no-op that looks like a working reload.
            Reload::HelmClients => Err(Error::Reload(
                "helm's control socket does not exist until M2".into(),
            )),
        }
    }
}

fn run(argv: &[String]) -> Result<()> {
    let Some((program, args)) = argv.split_first() else {
        return Err(Error::Reload("empty reload command".into()));
    };
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|e| Error::Reload(format!("the theme is applied, but {program} failed: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Reload(format!(
            "the theme is applied, but {program} exited with {status}"
        )))
    }
}

/// Signal every process whose `comm` is `process`.
///
/// No match is success, not failure: "foot is not running" is a perfectly good
/// outcome for a reload, and the next foot to start reads the new file anyway.
fn signal_all(process: &str, signal: i32) -> Result<()> {
    // rustix only offers an unsafe raw conversion, and this crate forbids
    // unsafe, so the signals helm is willing to send are named rather than
    // parsed. Widening the list is a deliberate act, which is the right shape
    // for "which signals may a theme apply send to a stranger's process".
    let signal = match signal {
        s if s == Signal::USR1.as_raw() => Signal::USR1,
        s if s == Signal::USR2.as_raw() => Signal::USR2,
        s if s == Signal::HUP.as_raw() => Signal::HUP,
        other => return Err(Error::Reload(format!("helm will not send signal {other}"))),
    };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        match std::fs::read_to_string(entry.path().join("comm")) {
            Ok(comm) if comm.trim() == process => {}
            // Processes exit between the readdir and the read; that is normal.
            _ => continue,
        }
        if let Some(pid) = Pid::from_raw(pid) {
            let _ = rustix::process::kill_process(pid, signal);
        }
    }
    Ok(())
}
