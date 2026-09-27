use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use realm_control::{production_runtime_dir, ClientError, IpcPathError};
use realm_core::ipc::{Request, Response, SessionHealth, PROTOCOL_VERSION};

use super::Env;

pub(crate) const CHECK_IDS: [&str; 32] = [
    "session/socket",
    "session/protocol-version",
    "session/degraded",
    "wm/attached",
    "wm/layer-shell",
    "wm/capabilities",
    "wm/protocol-version",
    "env/identity",
    "env/wayland-display/process",
    "env/wayland-display/systemd",
    "env/wayland-display/dbus",
    "env/desktop/systemd",
    "env/desktop/dbus",
    "env/agree",
    "env/stale",
    "env/cursor",
    "env/xwayland",
    "env/list-matches-entry",
    "units/target",
    "units/wm",
    "units/bar",
    "units/restart-policy",
    "units/idle-lock",
    "portal/answers",
    "portal/config",
    "portal/filechooser",
    "portal/screencast",
    "palette/lint",
    "theme/outputs",
    "fonts/glyphs",
    "fonts/attribution",
    "tools/floors",
];

const SESSION_ENV_VARS: [&str; 7] = [
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "XDG_SESSION_DESKTOP",
    "XDG_RUNTIME_DIR",
    "XCURSOR_THEME",
    "XCURSOR_SIZE",
];

fn env_value(env: &impl Env, name: &str) -> Option<String> {
    env.var_os(name)
        .map(|value| value.to_string_lossy().into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Ok,
    Warn,
    Fail,
    Skip,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Fail => "FAIL",
            Self::Skip => "skip",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Check {
    pub(crate) id: &'static str,
    pub(crate) group: &'static str,
    pub(crate) status: Status,
    pub(crate) summary: String,
    pub(crate) symptom: Option<String>,
    pub(crate) cause: Option<String>,
    pub(crate) remedy: Option<String>,
    pub(crate) data: serde_json::Value,
}

impl Check {
    fn new(id: &'static str, status: Status, summary: impl Into<String>) -> Self {
        let mut check = Self::skipped(id, "");
        check.status = status;
        check.summary = summary.into();
        check
    }

    fn skipped(id: &'static str, reason: &str) -> Self {
        let group = match id.split_once('/').unwrap().0 {
            "env" => "environment",
            group => group,
        };
        Self {
            id,
            group,
            status: Status::Skip,
            summary: reason.to_owned(),
            symptom: None,
            cause: None,
            remedy: None,
            data: serde_json::Value::Null,
        }
    }

    fn with_diagnostic(mut self, symptom: &str, cause: &str, remedy: &str) -> Self {
        self.symptom = Some(symptom.to_owned());
        self.cause = Some(cause.to_owned());
        self.remedy = Some(remedy.to_owned());
        self
    }
}

#[derive(Serialize)]
pub(crate) struct DoctorReport {
    tool: &'static str,
    version: &'static str,
    protocol: u32,
    #[serde(skip)]
    host: String,
    pub(crate) checks: Vec<Check>,
    summary: Summary,
}

#[derive(Serialize)]
struct Summary {
    ok: usize,
    warn: usize,
    fail: usize,
    skip: usize,
}

impl DoctorReport {
    fn new(checks: Vec<Check>) -> Self {
        debug_assert_eq!(
            checks.iter().map(|check| check.id).collect::<Vec<_>>(),
            CHECK_IDS
        );
        Self {
            tool: "realmctl doctor",
            version: env!("CARGO_PKG_VERSION"),
            protocol: realm_core::ipc::PROTOCOL_VERSION,
            host: host_summary(),
            summary: Summary {
                ok: checks
                    .iter()
                    .filter(|check| check.status == Status::Ok)
                    .count(),
                warn: checks
                    .iter()
                    .filter(|check| check.status == Status::Warn)
                    .count(),
                fail: checks
                    .iter()
                    .filter(|check| check.status == Status::Fail)
                    .count(),
                skip: checks
                    .iter()
                    .filter(|check| check.status == Status::Skip)
                    .count(),
            },
            checks,
        }
    }

    #[cfg(test)]
    fn fixture() -> Self {
        Self::new(
            CHECK_IDS
                .map(|id| Check::skipped(id, "fixture observation unavailable"))
                .into(),
        )
    }

    pub(crate) fn render_human(&self) -> String {
        use std::fmt::Write as _;
        let mut output = format!(
            "realmctl doctor - realmctl {}, protocol {}\n{}\n",
            self.version, self.protocol, self.host
        );
        if let (Some(wm), Some(session)) = (
            self.checks.iter().find(|check| check.id == "wm/attached"),
            self.checks
                .iter()
                .find(|check| check.id == "session/socket"),
        ) {
            if wm.status != Status::Skip {
                let _ = writeln!(output, "  {} | {}", wm.summary, session.summary);
            }
        }
        if self
            .checks
            .first()
            .is_some_and(|check| check.status == Status::Skip)
        {
            let _ = writeln!(
                output,
                "no realm session is running - {} of 32 checks skipped",
                self.summary.skip
            );
        }
        let mut group = "";
        for check in &self.checks {
            if check.group != group {
                group = check.group;
                let _ = writeln!(output, "\n{group}");
            }
            write_wrapped(
                &mut output,
                &format!("  {:<6} {:<32} ", check.status.label(), check.id),
                &check.summary,
            );
            for (label, detail) in [
                ("symptom", &check.symptom),
                ("cause", &check.cause),
                ("fix", &check.remedy),
            ] {
                if let Some(detail) = detail {
                    write_wrapped(&mut output, &format!("         {label:<8} "), detail);
                }
            }
        }
        let _ = writeln!(
            output,
            "32 checks: {} ok, {} warn, {} failed, {} skipped",
            self.summary.ok, self.summary.warn, self.summary.fail, self.summary.skip
        );
        output
    }

    pub(crate) fn render_json(&self) -> String {
        let mut output = serde_json::to_string(self).expect("doctor report is serializable");
        output.push('\n');
        output
    }

    pub(crate) fn failed(&self) -> bool {
        self.summary.fail != 0
    }
}

fn write_wrapped(output: &mut String, prefix: &str, value: &str) {
    use std::fmt::Write as _;

    const WIDTH: usize = 80;
    let continuation = " ".repeat(prefix.chars().count());
    let mut rest = value.trim();
    let mut first = true;
    while !rest.is_empty() {
        let current_prefix = if first { prefix } else { &continuation };
        let available = WIDTH.saturating_sub(current_prefix.chars().count()).max(1);
        let mut end = rest
            .char_indices()
            .nth(available)
            .map_or(rest.len(), |(index, _)| index);
        if end < rest.len() {
            if let Some(space) = rest[..end].rfind(char::is_whitespace) {
                if space != 0 {
                    end = space;
                }
            }
        }
        let _ = writeln!(output, "{current_prefix}{}", &rest[..end]);
        rest = rest[end..].trim_start();
        first = false;
    }
    if first {
        let _ = writeln!(output, "{prefix}");
    }
}

fn host_summary() -> String {
    let distribution = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=")
                    .map(|value| value.trim_matches('"').to_owned())
            })
        })
        .unwrap_or_else(|| "distribution unavailable".to_owned());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|_| "unavailable".to_owned());
    format!("{distribution} | kernel {kernel}")
}

pub(crate) fn run(
    env: &impl Env,
    json: bool,
    palette: Option<&Path>,
    portal_roundtrip: bool,
) -> ExitCode {
    let report = collect(env, palette, portal_roundtrip);
    if json {
        print!("{}", report.render_json());
    } else {
        print!("{}", report.render_human());
    }
    if report.failed() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn collect(env: &impl Env, palette: Option<&Path>, portal_roundtrip: bool) -> DoctorReport {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(3);
    let bus_deadline = started + Duration::from_secs(2);
    let bus_receiver = session_bus_present(env).then(|| begin_bus_observation(portal_roundtrip));
    let session_receiver = begin_session_observation();
    let mut checks: Vec<Check> = CHECK_IDS
        .into_iter()
        .map(|id| Check::skipped(id, "observation unavailable"))
        .collect();
    let session = finish_session_observation(session_receiver, deadline);
    let bus = bus_receiver.map_or_else(
        || BusObservation {
            systemd: Err("no session bus".to_owned()),
            portal: Err("no session bus".to_owned()),
        },
        |receiver| finish_bus_observation(receiver, bus_deadline),
    );
    apply_session_checks(&mut checks, &session);
    apply_environment_checks(&mut checks, env, session.is_live(), &bus, deadline);
    apply_deferred_checks(
        &mut checks,
        env,
        palette,
        portal_roundtrip,
        session.is_live(),
        &bus,
        deadline,
    );
    ensure_failure_diagnostics(&mut checks);
    DoctorReport::new(checks)
}

fn session_bus_present(env: &impl Env) -> bool {
    env_value(env, "DBUS_SESSION_BUS_ADDRESS").is_some()
        || env_value(env, "XDG_RUNTIME_DIR")
            .is_some_and(|root| Path::new(&root).join("bus").exists())
}

fn ensure_failure_diagnostics(checks: &mut [Check]) {
    for check in checks {
        if check.status != Status::Fail || check.symptom.is_some() {
            continue;
        }
        let (symptom, cause, remedy) = match check.id {
            "session/socket" => (
                "windows are unplaced and keys are dead",
                "the fixed control endpoint exists but is unusable",
                "restart the realm session from the display manager",
            ),
            "session/protocol-version" => (
                "the CLI and session would misread each other's frames",
                "realmctl and realm-wm came from different protocol bundles",
                "install matching realmctl and realm-wm packages",
            ),
            "wm/layer-shell" => (
                "the bar never appears",
                "realm-wm is not serving river-layer-shell-v1",
                "restart realm-wm and verify its supported River interfaces",
            ),
            "wm/protocol-version" => (
                "a River upgrade broke the session contract",
                "one or more negotiated interfaces are below the supported floor",
                "install the supported River 0.4.x package",
            ),
            "env/identity" => (
                "portals select the wrong backend and screen sharing fails",
                "the process does not carry Realm's desktop identity",
                "start the installed realm session from the display manager",
            ),
            "env/wayland-display/process" => (
                "this process cannot connect to the Realm display",
                "WAYLAND_DISPLAY or XDG_RUNTIME_DIR is absent",
                "run realmctl from a terminal inside the realm session",
            ),
            "env/wayland-display/systemd" | "env/desktop/systemd" => (
                "user units start without the graphical-session environment",
                "the session entry did not publish the observed process value",
                "restore the import-environment step in realm-session",
            ),
            "env/wayland-display/dbus" | "env/desktop/dbus" => (
                "file dialogs hang for about 25 seconds, then fail",
                "the activation environment, portal service, or selected backend may be broken",
                "run dbus-update-activation-environment --systemd before first bus use, then verify portal packages",
            ),
            "env/agree" => (
                "applications disagree about which graphical session is active",
                "the environment import ran early or with different values",
                "start a fresh realm login after clearing stale manager values",
            ),
            "env/stale" => (
                "user services try to connect to a dead display socket",
                "the user manager retained WAYLAND_DISPLAY across logout",
                "run systemctl --user unset-environment WAYLAND_DISPLAY, then log in again",
            ),
            "env/cursor" => (
                "the cursor is wrong, invisible, or changes size between surfaces",
                "process, systemd, GSettings, or installed cursor theme observations disagree",
                "install the selected cursor theme and restart the realm session",
            ),
            "env/list-matches-entry" => (
                "a required graphical variable is missing from some clients",
                "doctor and the installed session entry carry different import lists",
                "install matching realmctl and realm-session files",
            ),
            "units/target" => (
                "the session target starts nothing while reporting success",
                "the target is inactive or its wants links are missing",
                "reinstall the realm systemd user units and reload the user manager",
            ),
            "units/wm" => (
                "windows are never placed and keys are dead",
                "realm-wm.service is inactive or its condition failed",
                "inspect WAYLAND_DISPLAY and restart realm-wm.service",
            ),
            "units/bar" => (
                "the bar is gone without a visible explanation",
                "realm-bar.service is inactive",
                "restart realm-bar.service after realm-wm is active",
            ),
            "units/restart-policy" => (
                "a crashed desktop component is not supervised correctly",
                "installed unit restart policy differs from SPEC 0005",
                "reinstall matching realm systemd user units",
            ),
            "portal/answers" => (
                "Open File does nothing in applications",
                "the desktop portal did not answer its bounded property probe",
                "install and start xdg-desktop-portal with the gtk and wlr backends",
            ),
            "portal/config" => (
                "portal behavior changes with whichever backend is discovered",
                "the effective routes or advertised backend interfaces are missing",
                "install realm-portals.conf plus gtk.portal and wlr.portal metadata",
            ),
            "portal/filechooser" => (
                "the deliberately requested file dialog failed to open promptly",
                "FileChooser.OpenFile did not return a cancellable request handle",
                "repair the portal service, activation environment, and gtk backend",
            ),
            "portal/screencast" => (
                "screen sharing offers no sources",
                "the ScreenCast portal interface is unavailable",
                "install xdg-desktop-portal-wlr and the accepted portal routes",
            ),
            "palette/lint" => (
                "one or more Realm surfaces cannot use the selected palette",
                "the palette is unreadable or has fatal lint findings",
                "fix the reported palette findings, then run realmctl theme apply",
            ),
            _ => continue,
        };
        check.symptom = Some(symptom.to_owned());
        check.cause = Some(cause.to_owned());
        check.remedy = Some(remedy.to_owned());
    }
}

enum SessionObservation {
    Absent(String),
    Failed(String),
    Mismatch { client: u32, server: u32 },
    Live(SessionHealth),
}

fn begin_session_observation() -> mpsc::Receiver<SessionObservation> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(observe_session());
    });
    receiver
}

fn finish_session_observation(
    receiver: mpsc::Receiver<SessionObservation>,
    deadline: Instant,
) -> SessionObservation {
    match receiver.recv_timeout(
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(2)),
    ) {
        Ok(observation) => observation,
        Err(_) => SessionObservation::Failed("session probe exceeded 2000 ms".to_owned()),
    }
}

impl SessionObservation {
    fn is_live(&self) -> bool {
        matches!(self, Self::Live(_))
    }
}

fn observe_session() -> SessionObservation {
    let runtime = match production_runtime_dir() {
        Ok(runtime) => runtime,
        Err(IpcPathError::MissingRuntimeDir) => {
            return SessionObservation::Absent("XDG_RUNTIME_DIR is unavailable".to_owned())
        }
        Err(error) => return SessionObservation::Failed(error.to_string()),
    };
    let endpoint = runtime.client_endpoint();
    let start = Instant::now();
    let mut client = match super::retry::run(start, Instant::now, std::thread::sleep, || {
        endpoint.connect("realmctl")
    }) {
        Ok(client) => client,
        Err(ClientError::MissingRealm) => {
            return SessionObservation::Absent("no realm session is running".to_owned())
        }
        Err(ClientError::VersionMismatch { client, server }) => {
            return SessionObservation::Mismatch { client, server }
        }
        Err(error) => return SessionObservation::Failed(error.to_string()),
    };
    match client.request(Request::GetHealth) {
        Ok(Response::Health(health)) => SessionObservation::Live(*health),
        Ok(Response::Error { message, .. }) => SessionObservation::Failed(message),
        Ok(_) => SessionObservation::Failed("GetHealth returned an unexpected response".to_owned()),
        Err(error) => SessionObservation::Failed(error.to_string()),
    }
}

fn set(checks: &mut [Check], check: Check) {
    let slot = checks
        .iter_mut()
        .find(|candidate| candidate.id == check.id)
        .expect("every concrete doctor check has a fixed slot");
    *slot = check;
}

fn apply_session_checks(checks: &mut [Check], session: &SessionObservation) {
    match session {
        SessionObservation::Absent(reason) => {
            for id in &CHECK_IDS[..7] {
                set(checks, Check::skipped(id, reason));
            }
        }
        SessionObservation::Failed(reason) => {
            set(
                checks,
                Check::new("session/socket", Status::Fail, reason).with_diagnostic(
                    "windows are unplaced and keys are dead",
                    "the fixed control endpoint exists but is unusable",
                    "restart the realm session from the display manager",
                ),
            );
            for id in &CHECK_IDS[1..7] {
                set(checks, Check::skipped(id, "session health was unavailable"));
            }
        }
        SessionObservation::Mismatch { client, server } => {
            set(
                checks,
                Check::new(
                    "session/socket",
                    Status::Ok,
                    "control endpoint answered Hello",
                ),
            );
            set(
                checks,
                Check::new(
                    "session/protocol-version",
                    Status::Fail,
                    format!("client {client} != session {server}"),
                ),
            );
            for id in &CHECK_IDS[2..7] {
                set(
                    checks,
                    Check::skipped(id, "protocol mismatch prevented GetHealth"),
                );
            }
        }
        SessionObservation::Live(health) => {
            set(
                checks,
                Check::new(
                    "session/socket",
                    Status::Ok,
                    format!("realm-wm {} answered", health.session_version),
                ),
            );
            let protocol_status = if health.protocol_version == PROTOCOL_VERSION {
                Status::Ok
            } else {
                Status::Fail
            };
            set(
                checks,
                Check::new(
                    "session/protocol-version",
                    protocol_status,
                    format!("{} == {}", health.protocol_version, PROTOCOL_VERSION),
                ),
            );
            let degraded = match &health.degraded_codes {
                Some(codes) if codes.is_empty() => Check::new(
                    "session/degraded",
                    Status::Ok,
                    "no DEGRADED codes in this session",
                ),
                Some(codes) => Check::new(
                    "session/degraded",
                    Status::Warn,
                    format!("active codes: {}", codes.join(", ")),
                ),
                None => Check::new(
                    "session/degraded",
                    Status::Warn,
                    "current-session degradation handoff unavailable",
                ),
            };
            set(checks, degraded);
            set(
                checks,
                Check::new(
                    "wm/attached",
                    Status::Ok,
                    format!("{} holds River window management", health.backend_name),
                ),
            );
            set(
                checks,
                Check::new(
                    "wm/layer-shell",
                    if health.layer_shell_served {
                        Status::Ok
                    } else {
                        Status::Fail
                    },
                    if health.layer_shell_served {
                        "river-layer-shell-v1 is served"
                    } else {
                        "river-layer-shell-v1 is not served"
                    },
                ),
            );
            set(
                checks,
                Check::new(
                    "wm/capabilities",
                    if health.capabilities.unsupported.is_empty() {
                        Status::Ok
                    } else {
                        Status::Warn
                    },
                    if health.capabilities.unsupported.is_empty() {
                        format!("{} reports full MVP capabilities", health.backend_name)
                    } else {
                        format!(
                            "{} unsupported: {}",
                            health.backend_name,
                            health.capabilities.unsupported.join(", ")
                        )
                    },
                ),
            );
            let interfaces = health
                .bound_interfaces
                .iter()
                .map(|interface| format!("{} v{}", interface.name, interface.version))
                .collect::<Vec<_>>()
                .join(", ");
            let valid = [
                ("river_window_manager_v1", 4),
                ("river_xkb_bindings_v1", 3),
                ("river_layer_shell_v1", 1),
                ("river_input_manager_v1", 2),
                ("river_libinput_config_v1", 2),
            ]
            .into_iter()
            .all(|(name, floor)| {
                health
                    .bound_interfaces
                    .iter()
                    .any(|interface| interface.name == name && interface.version >= floor)
            });
            set(
                checks,
                Check::new(
                    "wm/protocol-version",
                    if valid { Status::Ok } else { Status::Fail },
                    interfaces,
                ),
            );
        }
    }
}

fn apply_environment_checks(
    checks: &mut [Check],
    env: &impl Env,
    session_live: bool,
    bus: &BusObservation,
    deadline: Instant,
) {
    let value = |name| env_value(env, name);
    let identity = [
        ("XDG_CURRENT_DESKTOP", "realm"),
        ("XDG_SESSION_TYPE", "wayland"),
        ("XDG_SESSION_DESKTOP", "realm"),
    ];
    let identity_ok = identity
        .iter()
        .all(|(name, expected)| value(name).as_deref() == Some(*expected));
    set(
        checks,
        Check::new(
            "env/identity",
            if identity_ok {
                Status::Ok
            } else if !session_live {
                Status::Skip
            } else {
                Status::Fail
            },
            identity
                .iter()
                .map(|(name, _)| {
                    format!("{name}={}", value(name).unwrap_or_else(|| "unset".into()))
                })
                .collect::<Vec<_>>()
                .join(" "),
        ),
    );
    let display = value("WAYLAND_DISPLAY");
    let runtime = value("XDG_RUNTIME_DIR");
    set(
        checks,
        Check::new(
            "env/wayland-display/process",
            if display.is_some() && runtime.is_some() {
                Status::Ok
            } else if session_live {
                Status::Fail
            } else {
                Status::Skip
            },
            format!(
                "WAYLAND_DISPLAY={} XDG_RUNTIME_DIR={}",
                display.as_deref().unwrap_or("unset"),
                runtime.as_deref().unwrap_or("unset")
            ),
        ),
    );
    let bus_present = session_bus_present(env);
    apply_systemd_environment_checks(
        checks,
        env,
        display.as_deref(),
        runtime.as_deref(),
        session_live,
        bus_present,
        &bus.systemd,
    );
    apply_portal_environment_checks(checks, session_live, bus_present, &bus.portal);
    set(checks, session_entry_import_check());
    set(
        checks,
        cursor_check(env, &bus.systemd, session_live, deadline),
    );
    set(checks, xwayland_check(env, &bus.systemd));
}

fn session_entry_import_check() -> Check {
    let installed_bin = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let fallbacks = [
        PathBuf::from("/usr/bin/realm-session"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/session/realm-session"),
    ];
    session_entry_import_check_from_installation(installed_bin.as_deref(), fallbacks)
}

fn session_entry_import_check_from_installation(
    installed_bin: Option<&Path>,
    fallbacks: impl IntoIterator<Item = PathBuf>,
) -> Check {
    if let Some(bin) = installed_bin {
        let wrapped = bin.join(".realm-session-wrapped");
        if wrapped.is_file() {
            return session_entry_import_check_from_candidates([wrapped]);
        }
        let direct = bin.join("realm-session");
        if direct.is_file() {
            return session_entry_import_check_from_candidates([direct]);
        }
    }
    session_entry_import_check_from_candidates(fallbacks)
}

fn session_entry_import_check_from_candidates(
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Check {
    for path in candidates {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let parsed = parse_session_imports(&contents);
        if parsed.as_deref() == Some(SESSION_ENV_VARS.as_slice()) {
            return Check::new(
                "env/list-matches-entry",
                Status::Ok,
                format!("{} matches the accepted import list", path.display()),
            );
        }
        return Check::new(
            "env/list-matches-entry",
            Status::Fail,
            format!("{} import list differs or is malformed", path.display()),
        );
    }
    Check::skipped(
        "env/list-matches-entry",
        "installed session entry was not available for the CI import-list check",
    )
}

fn cursor_check(
    env: &impl Env,
    systemd: &Result<SystemdObservation, String>,
    session_live: bool,
    deadline: Instant,
) -> Check {
    let process_theme = env_value(env, "XCURSOR_THEME");
    let process_size = env_value(env, "XCURSOR_SIZE");
    let manager_theme = systemd
        .as_ref()
        .ok()
        .and_then(|value| value.environment.get("XCURSOR_THEME"));
    let manager_size = systemd
        .as_ref()
        .ok()
        .and_then(|value| value.environment.get("XCURSOR_SIZE"));
    let settings_theme = command_until(
        deadline,
        "gsettings",
        &["get", "org.gnome.desktop.interface", "cursor-theme"],
    )
    .and_then(|value| {
        let value = value.trim_matches(['\'', '"']);
        if value.is_empty() {
            Err("malformed empty cursor-theme".to_owned())
        } else {
            Ok(value.to_owned())
        }
    });
    let settings_size = command_until(
        deadline,
        "gsettings",
        &["get", "org.gnome.desktop.interface", "cursor-size"],
    )
    .and_then(|value| match value.parse::<u32>() {
        Ok(size) if size > 0 => Ok(value),
        _ => Err("malformed cursor-size".to_owned()),
    });
    let theme_resolves = process_theme
        .as_deref()
        .is_some_and(|theme| cursor_theme_resolves(env, theme));
    let process_size_valid = process_size
        .as_deref()
        .and_then(|value| value.parse::<u32>().ok())
        .is_some_and(|size| size > 0);
    let agrees = process_theme.as_ref() == manager_theme
        && process_size.as_ref() == manager_size
        && settings_theme.as_ref().ok() == process_theme.as_ref()
        && settings_size.as_ref().ok() == process_size.as_ref();
    let complete = process_theme.is_some()
        && process_size.is_some()
        && manager_theme.is_some()
        && manager_size.is_some()
        && settings_theme.is_ok()
        && settings_size.is_ok()
        && theme_resolves
        && process_size_valid;
    let status = if complete && agrees {
        Status::Ok
    } else if session_live {
        Status::Fail
    } else {
        Status::Warn
    };
    let settings_theme_summary = settings_theme
        .as_deref()
        .map_or_else(|error| format!("error:{error}"), ToOwned::to_owned);
    let settings_size_summary = settings_size
        .as_deref()
        .map_or_else(|error| format!("error:{error}"), ToOwned::to_owned);
    Check::new(
        "env/cursor",
        status,
        format!(
            "process={}/{} systemd={}/{} gsettings={}/{} theme-resolves={theme_resolves}",
            process_theme.as_deref().unwrap_or("unset"),
            process_size.as_deref().unwrap_or("unset"),
            manager_theme.map(String::as_str).unwrap_or("unavailable"),
            manager_size.map(String::as_str).unwrap_or("unavailable"),
            settings_theme_summary,
            settings_size_summary,
        ),
    )
}

fn cursor_theme_resolves(env: &impl Env, theme: &str) -> bool {
    let mut roots = Vec::new();
    if let Some(root) = env_value(env, "XDG_DATA_HOME") {
        roots.push(PathBuf::from(root).join("icons"));
    } else if let Some(home) = env_value(env, "HOME") {
        roots.push(PathBuf::from(&home).join(".local/share/icons"));
    }
    if let Some(home) = env_value(env, "HOME") {
        roots.push(PathBuf::from(home).join(".icons"));
    }
    let data_dirs =
        env_value(env, "XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    roots.extend(
        data_dirs
            .split(':')
            .filter(|root| !root.is_empty())
            .map(|root| Path::new(root).join("icons")),
    );
    roots
        .into_iter()
        .any(|root| root.join(theme).join("cursors").is_dir())
}

fn xwayland_check(env: &impl Env, systemd: &Result<SystemdObservation, String>) -> Check {
    let process = env_value(env, "DISPLAY");
    let manager = systemd
        .as_ref()
        .ok()
        .and_then(|value| value.environment.get("DISPLAY"));
    let socket = process.as_deref().and_then(x11_socket_path);
    let answers = socket
        .as_deref()
        .is_some_and(|path| UnixStream::connect(path).is_ok());
    Check::new(
        "env/xwayland",
        Status::Warn,
        format!(
            "process={} systemd={} socket={} answers={answers}; activation value unobservable",
            process.as_deref().unwrap_or("unset"),
            manager.map(String::as_str).unwrap_or("unavailable"),
            socket.as_deref().map_or_else(
                || "unavailable".to_owned(),
                |path| path.display().to_string()
            ),
        ),
    )
}

fn x11_socket_path(display: &str) -> Option<PathBuf> {
    let number = display.strip_prefix(':')?.split('.').next()?;
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(Path::new("/tmp/.X11-unix").join(format!("X{number}")))
}

fn command_until(deadline: Instant, program: &str, args: &[&str]) -> Result<String, String> {
    if Instant::now() >= deadline {
        return Err("deadline exceeded".to_owned());
    }
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "not found".to_owned()
            } else {
                error.to_string()
            }
        })?;
    enum PipeOutput {
        Stdout(Result<String, String>),
        Stderr(Result<String, String>),
    }
    let (pipe_sender, pipe_receiver) = mpsc::sync_channel(2);
    if let Some(mut pipe) = child.stdout.take() {
        let sender = pipe_sender.clone();
        std::thread::spawn(move || {
            let mut value = String::new();
            let result = pipe
                .read_to_string(&mut value)
                .map(|_| value)
                .map_err(|error| error.to_string());
            let _ = sender.send(PipeOutput::Stdout(result));
        });
    }
    if let Some(mut pipe) = child.stderr.take() {
        let sender = pipe_sender.clone();
        std::thread::spawn(move || {
            let mut value = String::new();
            let result = pipe
                .read_to_string(&mut value)
                .map(|_| value)
                .map_err(|error| error.to_string());
            let _ = sender.send(PipeOutput::Stderr(result));
        });
    }
    drop(pipe_sender);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(10)),
                );
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".to_owned());
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    for _ in 0..2 {
        let output = pipe_receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "timed out reading command output".to_owned())?;
        match output {
            PipeOutput::Stdout(result) => stdout = result?,
            PipeOutput::Stderr(result) => stderr = result?,
        }
    }
    if status.success() {
        Ok(stdout.trim().to_owned())
    } else {
        Err(stderr
            .lines()
            .next()
            .filter(|line| !line.is_empty())
            .unwrap_or("command failed")
            .to_owned())
    }
}

fn apply_deferred_checks(
    checks: &mut [Check],
    env: &impl Env,
    palette: Option<&Path>,
    portal_roundtrip: bool,
    session_live: bool,
    bus: &BusObservation,
    deadline: Instant,
) {
    let reason = if session_live {
        "probe unavailable"
    } else {
        "no realm session is running"
    };
    for id in &CHECK_IDS[18..27] {
        set(checks, Check::skipped(id, reason));
    }
    set(
        checks,
        Check::skipped(
            "units/idle-lock",
            "locker selection remains an accepted needs-human decision",
        ),
    );
    if !portal_roundtrip {
        set(
            checks,
            Check::skipped(
                "portal/filechooser",
                "not requested; pass --portal-roundtrip to open a bounded dialog",
            ),
        );
    }
    apply_unit_checks(checks, session_live, &bus.systemd);
    apply_portal_checks(checks, env, session_live, &bus.portal, portal_roundtrip);
    let config_root = super::default_config_root(env).ok();
    let palette_result: Result<_, String> = match palette {
        Some(path) => realm_core::Palette::load(path)
            .map(|value| (value, path.display().to_string()))
            .map_err(|error| error.to_string()),
        None => config_root.as_deref().map_or_else(
            || {
                realm_core::Palette::from_toml(realm_theme::SHIPPED_PALETTE)
                    .map(|value| (value, "shipped palette".to_owned()))
                    .map_err(|error| error.to_string())
            },
            |root| {
                realm_theme::load_lint_palette(root)
                    .map(|value| {
                        let user = root.join(realm_theme::USER_PALETTE);
                        (
                            value,
                            if user.exists() {
                                user.display().to_string()
                            } else {
                                "shipped palette".to_owned()
                            },
                        )
                    })
                    .map_err(|error| error.to_string())
            },
        ),
    };
    match &palette_result {
        Ok((palette, source)) => {
            let findings = palette.lint();
            let fatal = findings.iter().any(|finding| finding.fatal);
            set(
                checks,
                Check::new(
                    "palette/lint",
                    if fatal {
                        Status::Fail
                    } else if findings.is_empty() {
                        Status::Ok
                    } else {
                        Status::Warn
                    },
                    if findings.is_empty() {
                        format!("{source} parses and passes lint")
                    } else {
                        findings
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join("; ")
                    },
                ),
            );
        }
        Err(error) => set(checks, Check::new("palette/lint", Status::Fail, error)),
    }
    set(checks, theme_outputs_check(config_root.as_deref(), palette));
    let palette_for_probe = palette_result.ok().map(|(palette, _)| palette);
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let fonts = palette_for_probe.as_ref().map(font_checks);
        let tools = tool_floor_check([
            ("yazi", command_until(deadline, "yazi", &["--version"])),
            ("btop", command_until(deadline, "btop", &["--version"])),
            (
                "starship",
                command_until(deadline, "starship", &["--version"]),
            ),
        ]);
        let _ = sender.send((fonts, tools));
    });
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok((Some((glyphs, attribution)), tools)) => {
            set(checks, glyphs);
            set(checks, attribution);
            set(checks, tools);
        }
        Ok((None, tools)) => {
            set(
                checks,
                Check::skipped("fonts/glyphs", "palette was unavailable"),
            );
            set(
                checks,
                Check::skipped("fonts/attribution", "palette was unavailable"),
            );
            set(checks, tools);
        }
        Err(_) => {
            set(
                checks,
                Check::new("fonts/glyphs", Status::Warn, "font probe exceeded deadline"),
            );
            set(
                checks,
                Check::new(
                    "fonts/attribution",
                    Status::Warn,
                    "font probe exceeded deadline",
                ),
            );
            set(
                checks,
                Check::new(
                    "tools/floors",
                    Status::Warn,
                    "tool probes exceeded deadline",
                ),
            );
        }
    }
}

#[derive(Debug)]
struct SystemdUnit {
    active_state: String,
    condition_result: Option<bool>,
    restarts: Option<u32>,
}

#[derive(Debug)]
struct SystemdObservation {
    environment: BTreeMap<String, String>,
    target: SystemdUnit,
    wm: SystemdUnit,
    bar: SystemdUnit,
}

#[derive(Debug)]
struct PortalObservation {
    filechooser_version: u32,
    screencast_version: Result<u32, String>,
    filechooser_roundtrip: Option<Result<(), String>>,
}

struct BusObservation {
    systemd: Result<SystemdObservation, String>,
    portal: Result<PortalObservation, String>,
}

struct BusReceivers {
    systemd: mpsc::Receiver<Result<SystemdObservation, String>>,
    portal: mpsc::Receiver<Result<PortalObservation, String>>,
}

fn begin_bus_observation(portal_roundtrip: bool) -> BusReceivers {
    begin_bus_observation_with(observe_systemd, move || observe_portal(portal_roundtrip))
}

fn begin_bus_observation_with<SystemdProbe, PortalProbe>(
    systemd_probe: SystemdProbe,
    portal_probe: PortalProbe,
) -> BusReceivers
where
    SystemdProbe: FnOnce() -> Result<SystemdObservation, String> + Send + 'static,
    PortalProbe: FnOnce() -> Result<PortalObservation, String> + Send + 'static,
{
    let (systemd_sender, systemd) = mpsc::sync_channel(1);
    let (portal_sender, portal) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = systemd_sender.send(systemd_probe());
    });
    std::thread::spawn(move || {
        let _ = portal_sender.send(portal_probe());
    });
    BusReceivers { systemd, portal }
}

fn finish_bus_observation(receivers: BusReceivers, deadline: Instant) -> BusObservation {
    fn receive<T>(
        receiver: mpsc::Receiver<Result<T, String>>,
        deadline: Instant,
        timeout: &str,
    ) -> Result<T, String> {
        receiver
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|_| Err(timeout.to_owned()))
    }
    let systemd = receive(
        receivers.systemd,
        deadline,
        "session D-Bus probe exceeded 2000 ms",
    );
    let portal = receive(
        receivers.portal,
        deadline,
        "portal proxy did not answer within 2000 ms",
    );
    BusObservation { systemd, portal }
}

fn observe_systemd() -> Result<SystemdObservation, String> {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedObjectPath;

    let connection = Connection::session().map_err(|error| error.to_string())?;
    let manager = Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .map_err(|error| error.to_string())?;
    let environment: Vec<String> = manager
        .get_property("Environment")
        .map_err(|error| error.to_string())?;
    let environment = environment
        .into_iter()
        .filter_map(|entry| {
            entry
                .split_once('=')
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
        })
        .collect();

    fn unit(
        connection: &Connection,
        manager: &Proxy<'_>,
        name: &str,
    ) -> Result<SystemdUnit, String> {
        let path: OwnedObjectPath = manager
            .call("GetUnit", &(name,))
            .map_err(|error| error.to_string())?;
        let proxy = Proxy::new(
            connection,
            "org.freedesktop.systemd1",
            path.as_str(),
            "org.freedesktop.systemd1.Unit",
        )
        .map_err(|error| error.to_string())?;
        let restarts = if name.ends_with(".service") {
            Proxy::new(
                connection,
                "org.freedesktop.systemd1",
                path.as_str(),
                "org.freedesktop.systemd1.Service",
            )
            .ok()
            .and_then(|service| service.get_property("NRestarts").ok())
        } else {
            None
        };
        Ok(SystemdUnit {
            active_state: proxy
                .get_property("ActiveState")
                .map_err(|error| error.to_string())?,
            condition_result: proxy.get_property("ConditionResult").ok(),
            restarts,
        })
    }

    Ok(SystemdObservation {
        environment,
        target: unit(&connection, &manager, "realm-session.target")?,
        wm: unit(&connection, &manager, "realm-wm.service")?,
        bar: unit(&connection, &manager, "realm-bar.service")?,
    })
}

fn observe_portal(portal_roundtrip: bool) -> Result<PortalObservation, String> {
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    let connection = Connection::session().map_err(|error| error.to_string())?;
    let filechooser = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.FileChooser",
    )
    .map_err(|error| error.to_string())?;
    let filechooser_version = filechooser
        .get_property("version")
        .map_err(|error| error.to_string())?;
    let screencast = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.ScreenCast",
    )
    .map_err(|error| error.to_string())?;
    let filechooser_roundtrip = portal_roundtrip.then(|| {
        let options: HashMap<String, OwnedValue> = HashMap::new();
        let request_path: OwnedObjectPath = filechooser
            .call("OpenFile", &("", "realmctl doctor", options))
            .map_err(|error| error.to_string())?;
        let request = Proxy::new(
            &connection,
            "org.freedesktop.portal.Desktop",
            request_path.as_str(),
            "org.freedesktop.portal.Request",
        )
        .map_err(|error| error.to_string())?;
        request
            .call::<_, _, ()>("Close", &())
            .map_err(|error| error.to_string())
    });
    Ok(PortalObservation {
        filechooser_version,
        screencast_version: screencast
            .get_property("version")
            .map_err(|error| error.to_string()),
        filechooser_roundtrip,
    })
}

fn apply_systemd_environment_checks(
    checks: &mut [Check],
    env: &impl Env,
    display: Option<&str>,
    runtime: Option<&str>,
    session_live: bool,
    bus_present: bool,
    observation: &Result<SystemdObservation, String>,
) {
    let unavailable = |id, reason: &str| {
        Check::new(
            id,
            if session_live {
                Status::Fail
            } else {
                Status::Skip
            },
            reason,
        )
    };
    if !bus_present {
        for id in [
            "env/wayland-display/systemd",
            "env/desktop/systemd",
            "env/agree",
            "env/stale",
        ] {
            set(checks, unavailable(id, "no session bus"));
        }
        return;
    }
    let systemd = match observation {
        Ok(systemd) => systemd,
        Err(error) => {
            for id in [
                "env/wayland-display/systemd",
                "env/desktop/systemd",
                "env/agree",
                "env/stale",
            ] {
                set(checks, unavailable(id, error));
            }
            return;
        }
    };
    let systemd_display = systemd.environment.get("WAYLAND_DISPLAY");
    set(
        checks,
        Check::new(
            "env/wayland-display/systemd",
            if display.is_some() && systemd_display.map(String::as_str) == display {
                Status::Ok
            } else {
                Status::Fail
            },
            format!(
                "process={} systemd={}",
                display.unwrap_or("unset"),
                systemd_display.map(String::as_str).unwrap_or("unset")
            ),
        ),
    );
    let systemd_desktop = systemd.environment.get("XDG_CURRENT_DESKTOP");
    set(
        checks,
        Check::new(
            "env/desktop/systemd",
            if systemd_desktop.map(String::as_str) == Some("realm") {
                Status::Ok
            } else {
                Status::Fail
            },
            format!(
                "XDG_CURRENT_DESKTOP={}",
                systemd_desktop.map(String::as_str).unwrap_or("unset")
            ),
        ),
    );
    const AGREEMENT_NAMES: [&str; 7] = [
        "WAYLAND_DISPLAY",
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_TYPE",
        "XDG_SESSION_DESKTOP",
        "XDG_RUNTIME_DIR",
        "XCURSOR_THEME",
        "XCURSOR_SIZE",
    ];
    let disagreements = AGREEMENT_NAMES
        .into_iter()
        .filter(|name| systemd.environment.get(*name).cloned() != env_value(env, name))
        .collect::<Vec<_>>();
    set(
        checks,
        Check::new(
            "env/agree",
            if disagreements.is_empty() {
                Status::Ok
            } else {
                Status::Fail
            },
            if disagreements.is_empty() {
                "process and systemd values agree; D-Bus values are unobservable".to_owned()
            } else {
                format!(
                    "process and systemd disagree for {}; D-Bus values are unobservable",
                    disagreements.join(", ")
                )
            },
        ),
    );
    let stale = match (runtime, systemd_display) {
        (Some(root), Some(name)) => Some(!Path::new(root).join(name).exists()),
        _ => None,
    };
    set(
        checks,
        Check::new(
            "env/stale",
            match stale {
                Some(false) => Status::Ok,
                Some(true) => Status::Fail,
                None if session_live => Status::Fail,
                None => Status::Skip,
            },
            match stale {
                Some(true) => "systemd WAYLAND_DISPLAY names no socket",
                Some(false) => "systemd display is not stale",
                None => "systemd display or runtime directory is unavailable",
            },
        ),
    );
}

fn apply_portal_environment_checks(
    checks: &mut [Check],
    session_live: bool,
    bus_present: bool,
    observation: &Result<PortalObservation, String>,
) {
    set(
        checks,
        portal_proxy_check(
            session_live,
            bus_present,
            observation,
            "env/wayland-display/dbus",
        ),
    );
    set(
        checks,
        portal_proxy_check(session_live, bus_present, observation, "env/desktop/dbus"),
    );
}

fn portal_proxy_check(
    session_live: bool,
    bus_present: bool,
    observation: &Result<PortalObservation, String>,
    id: &'static str,
) -> Check {
    let (failed, summary) = if !bus_present {
        (true, "no session bus".to_owned())
    } else {
        match observation {
            Ok(portal) => (
                false,
                format!(
                    "portal FileChooser v{} answered; activation value unobservable",
                    portal.filechooser_version
                ),
            ),
            Err(error) => (true, error.clone()),
        }
    };
    let status = if !failed {
        Status::Ok
    } else if session_live {
        Status::Fail
    } else {
        Status::Skip
    };
    let check = Check::new(id, status, summary);
    if failed {
        check.with_diagnostic(
            "file dialogs hang for about 25 seconds, then fail",
            "the activation environment, portal service, or selected backend may be broken",
            "before first bus use run dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP XDG_RUNTIME_DIR; then verify portal packages",
        )
    } else {
        check
    }
}

fn parse_session_imports(contents: &str) -> Option<Vec<&str>> {
    let mut in_list = false;
    let mut values = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if !in_list {
            if line == "SESSION_ENV_VARS=(" {
                in_list = true;
            }
            continue;
        }
        if line == ")" {
            return Some(values);
        }
        let value = line.split('#').next().unwrap_or_default().trim();
        if !value.is_empty() {
            values.push(value);
        }
    }
    None
}

fn tool_floor_check<const N: usize>(observations: [(&str, Result<String, String>); N]) -> Check {
    let mut reported = Vec::with_capacity(N);
    let mut missing = false;
    for (name, result) in observations {
        match result {
            Ok(version) if version.chars().any(|ch| ch.is_ascii_digit()) => {
                reported.push(format!("{name}: {version}"));
            }
            Ok(_) => {
                missing = true;
                reported.push(format!("{name}: unparseable version"));
            }
            Err(error) => {
                missing = true;
                reported.push(format!("{name}: {error}"));
            }
        }
    }
    let suffix = if missing {
        "install the missing or unparseable tools"
    } else {
        "numeric floors unresolved"
    };
    Check::new(
        "tools/floors",
        if missing { Status::Warn } else { Status::Skip },
        format!("{}; {suffix}", reported.join(", ")),
    )
}

fn theme_outputs_check(config_root: Option<&Path>, palette: Option<&Path>) -> Check {
    let Some(root) = config_root else {
        return Check::skipped(
            "theme/outputs",
            "no XDG_CONFIG_HOME or HOME for the theme comparison",
        );
    };
    let result = match palette {
        Some(path) => std::fs::read(path)
            .map_err(|error| error.to_string())
            .and_then(|raw_palette| {
                realm_theme::diff_with_snapshot(root, || {
                    realm_theme::ThemeSnapshot::new(
                        raw_palette,
                        b"realm-theme-launch-profile-v1\nnone\n".to_vec(),
                        BTreeMap::new(),
                        realm_theme::templates(),
                    )
                })
                .map_err(|error| error.to_string())
            }),
        None => realm_theme::diff(root).map_err(|error| error.to_string()),
    };
    match result {
        Ok(changes) if changes.is_empty() => {
            Check::new("theme/outputs", Status::Ok, "current generation matches")
        }
        Ok(changes) => Check::new(
            "theme/outputs",
            Status::Warn,
            format!(
                "{} generated outputs differ; run realmctl theme apply",
                changes.len()
            ),
        ),
        Err(error) => Check::new(
            "theme/outputs",
            Status::Warn,
            format!("theme comparison unavailable: {error}"),
        ),
    }
}

fn font_checks(palette: &realm_core::Palette) -> (Check, Check) {
    use fontdb::{Database, Family, Query, Stretch, Style, Weight};

    let mut db = Database::new();
    db.load_system_fonts();
    let mut chain = vec![palette.typography.family.clone()];
    for family in &palette.typography.fallback {
        if !chain.contains(family) {
            chain.push(family.clone());
        }
    }
    let faces = chain
        .iter()
        .filter_map(|requested| {
            let families = [Family::Name(requested)];
            db.query(&Query {
                families: &families,
                weight: Weight::NORMAL,
                stretch: Stretch::Normal,
                style: Style::Normal,
            })
            .map(|id| {
                let actual = db
                    .face(id)
                    .and_then(|face| face.families.first())
                    .map(|(name, _)| name.clone())
                    .unwrap_or_else(|| requested.clone());
                (id, requested.clone(), actual)
            })
        })
        .collect::<Vec<_>>();
    let covers = |id, ch| {
        db.with_face_data(id, |data, face_index| {
            ttf_parser::Face::parse(data, face_index)
                .ok()
                .and_then(|face| face.glyph_index(ch))
                .is_some()
        })
        .unwrap_or(false)
    };
    let probe = realm_core::glyphs::Probe::run(|ch| faces.iter().any(|(id, _, _)| covers(*id, ch)));
    let glyphs = Check::new(
        "fonts/glyphs",
        if probe.missing.is_empty() {
            Status::Ok
        } else {
            Status::Warn
        },
        probe.summary(),
    );
    let glyphs = if probe.missing.is_empty() {
        glyphs
    } else {
        glyphs.with_diagnostic(
            "orbit runes degrade to digits and other symbols use ASCII substitutes",
            "the selected font chain does not cover every realm glyph",
            "install Symbols Nerd Font Mono or Symbola",
        )
    };

    let risky = palette
        .glyphs
        .runes
        .iter()
        .chain(std::iter::once(&palette.glyphs.prompt_sigil))
        .filter_map(|glyph| glyph.chars().next())
        .collect::<Vec<_>>();
    let mut outside = false;
    let attribution = risky
        .into_iter()
        .map(|ch| {
            let supplier = faces
                .iter()
                .find(|(id, _, _)| covers(*id, ch))
                .map(|(_, requested, actual)| {
                    if actual != requested && !chain.contains(actual) {
                        outside = true;
                    }
                    format!("{ch}<-{actual}")
                })
                .unwrap_or_else(|| format!("{ch}<-missing"));
            supplier
        })
        .collect::<Vec<_>>()
        .join(", ");
    let attribution = Check::new(
        "fonts/attribution",
        if outside { Status::Warn } else { Status::Ok },
        attribution,
    );
    (glyphs, attribution)
}

fn apply_unit_checks(
    checks: &mut [Check],
    session_live: bool,
    observation: &Result<SystemdObservation, String>,
) {
    let systemd = match observation {
        Ok(systemd) => systemd,
        Err(error) => {
            for id in ["units/target", "units/wm", "units/bar"] {
                set(
                    checks,
                    Check::new(
                        id,
                        if session_live {
                            Status::Fail
                        } else {
                            Status::Skip
                        },
                        error,
                    ),
                );
            }
            apply_restart_policy_check(checks, session_live);
            return;
        }
    };
    set(checks, {
        let mut target = unit_active_check("units/target", &systemd.target, false);
        if !target_wants_installed() {
            target.status = Status::Fail;
            target.summary.push_str(" wants-links=missing");
        } else {
            target.summary.push_str(" wants-links=present");
        }
        target
    });
    set(checks, unit_active_check("units/wm", &systemd.wm, false));
    set(checks, unit_active_check("units/bar", &systemd.bar, true));
    apply_restart_policy_check(checks, session_live);
}

fn target_wants_installed() -> bool {
    [
        Path::new("/etc/systemd/user"),
        Path::new("/usr/lib/systemd/user"),
        Path::new("/lib/systemd/user"),
    ]
    .into_iter()
    .any(|root| {
        let wants = root.join("realm-session.target.wants");
        wants.join("realm-wm.service").exists() && wants.join("realm-bar.service").exists()
    })
}

fn unit_active_check(id: &'static str, unit: &SystemdUnit, restarting_ok: bool) -> Check {
    let active = unit.active_state == "active"
        || (restarting_ok && matches!(unit.active_state.as_str(), "activating" | "reloading"));
    let mut detail = format!("ActiveState={}", unit.active_state);
    if let Some(condition) = unit.condition_result {
        detail.push_str(&format!(" ConditionResult={condition}"));
    }
    if let Some(restarts) = unit.restarts {
        detail.push_str(&format!(" NRestarts={restarts}"));
    }
    Check::new(id, if active { Status::Ok } else { Status::Fail }, detail)
}

fn apply_restart_policy_check(checks: &mut [Check], session_live: bool) {
    let candidates = [
        Path::new("/etc/systemd/user"),
        Path::new("/usr/lib/systemd/user"),
        Path::new("/lib/systemd/user"),
    ];
    let read = |name: &str| {
        candidates
            .iter()
            .map(|root| root.join(name))
            .find_map(|path| std::fs::read_to_string(path).ok())
    };
    let valid = read("realm-wm.service")
        .zip(read("realm-bar.service"))
        .is_some_and(|(wm, bar)| restart_policy_valid(&wm, &bar));
    set(
        checks,
        Check::new(
            "units/restart-policy",
            if valid {
                Status::Ok
            } else if session_live {
                Status::Fail
            } else {
                Status::Skip
            },
            if valid {
                "installed WM and bar restart policies match SPEC 0005"
            } else {
                "installed WM or bar restart policy is missing or differs"
            },
        ),
    );
}

fn restart_policy_valid(wm: &str, bar: &str) -> bool {
    wm.lines().any(|line| line == "Restart=always")
        && wm
            .lines()
            .any(|line| line == "RestartPreventExitStatus=69 78")
        && wm.lines().any(|line| line == "StartLimitBurst=5")
        && bar.lines().any(|line| line == "Restart=on-failure")
        && bar.lines().any(|line| line == "StartLimitBurst=5")
}

fn apply_portal_checks(
    checks: &mut [Check],
    env: &impl Env,
    session_live: bool,
    observation: &Result<PortalObservation, String>,
    portal_roundtrip: bool,
) {
    match observation {
        Ok(portal) => {
            set(
                checks,
                Check::new(
                    "portal/answers",
                    Status::Ok,
                    format!("FileChooser v{} answered", portal.filechooser_version),
                ),
            );
            set(
                checks,
                match &portal.screencast_version {
                    Ok(version) => Check::new(
                        "portal/screencast",
                        Status::Ok,
                        format!("ScreenCast v{version} is available"),
                    ),
                    Err(error) => Check::new("portal/screencast", Status::Fail, error),
                },
            );
        }
        Err(error) => {
            for id in ["portal/answers", "portal/screencast"] {
                set(
                    checks,
                    Check::new(
                        id,
                        if session_live {
                            Status::Fail
                        } else {
                            Status::Skip
                        },
                        error,
                    ),
                );
            }
        }
    }
    apply_portal_config_check(checks, env);
    if portal_roundtrip {
        let result = observation
            .as_ref()
            .ok()
            .and_then(|portal| portal.filechooser_roundtrip.as_ref());
        set(
            checks,
            match result {
                Some(Ok(())) => Check::new(
                    "portal/filechooser",
                    Status::Ok,
                    "OpenFile returned a request handle and Close was sent",
                ),
                Some(Err(error)) => Check::new("portal/filechooser", Status::Fail, error),
                None => Check::new(
                    "portal/filechooser",
                    if session_live {
                        Status::Fail
                    } else {
                        Status::Skip
                    },
                    "portal round trip was unavailable",
                ),
            },
        );
    }
}

fn apply_portal_config_check(checks: &mut [Check], env: &impl Env) {
    let config_home = env_value(env, "XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env_value(env, "HOME").map(|home| Path::new(&home).join(".config")));
    let candidates = [
        config_home
            .as_ref()
            .map(|root| root.join("xdg-desktop-portal/realm-portals.conf")),
        config_home
            .as_ref()
            .map(|root| root.join("xdg-desktop-portal/portals.conf")),
        Some(PathBuf::from(
            "/etc/xdg/xdg-desktop-portal/realm-portals.conf",
        )),
        Some(PathBuf::from(
            "/usr/share/xdg-desktop-portal/realm-portals.conf",
        )),
    ];
    let found = candidates.into_iter().flatten().find_map(|path| {
        std::fs::read_to_string(&path)
            .ok()
            .map(|contents| (path, contents))
    });
    let routes = found.as_ref().is_some_and(|(_, contents)| {
        contents.contains("default=gtk")
            && contents.contains("org.freedesktop.impl.portal.ScreenCast=wlr")
            && contents.contains("org.freedesktop.impl.portal.Settings=gtk")
            && contents.contains("org.freedesktop.impl.portal.Screenshot=wlr")
            && contents.contains("org.freedesktop.impl.portal.Inhibit=none")
    });
    let metadata = portal_metadata_supports(
        env,
        "gtk",
        &[
            "org.freedesktop.impl.portal.FileChooser",
            "org.freedesktop.impl.portal.Settings",
        ],
    ) && portal_metadata_supports(
        env,
        "wlr",
        &[
            "org.freedesktop.impl.portal.ScreenCast",
            "org.freedesktop.impl.portal.Screenshot",
        ],
    );
    let valid = routes && metadata;
    set(
        checks,
        Check::new(
            "portal/config",
            if valid { Status::Ok } else { Status::Fail },
            match found {
                Some((path, _)) if valid => {
                    format!("{} selects installed gtk and wlr backends", path.display())
                }
                Some((path, _)) if !routes => {
                    format!("{} lacks required interface routes", path.display())
                }
                Some((path, _)) => format!(
                    "{} routes to missing or incomplete .portal metadata",
                    path.display()
                ),
                None => "no effective realm-portals.conf found".to_owned(),
            },
        ),
    );
}

fn portal_metadata_supports(env: &impl Env, backend: &str, interfaces: &[&str]) -> bool {
    let mut roots = Vec::new();
    if let Some(root) = env_value(env, "XDG_DATA_HOME") {
        roots.push(PathBuf::from(root));
    }
    let data_dirs =
        env_value(env, "XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    roots.extend(
        data_dirs
            .split(':')
            .filter(|root| !root.is_empty())
            .map(PathBuf::from),
    );
    roots.into_iter().any(|root| {
        let path = root
            .join("xdg-desktop-portal/portals")
            .join(format!("{backend}.portal"));
        std::fs::read_to_string(path).ok().is_some_and(|contents| {
            let advertised = contents
                .lines()
                .find_map(|line| line.strip_prefix("Interfaces="))
                .unwrap_or_default();
            interfaces
                .iter()
                .all(|interface| advertised.split(';').any(|value| value == *interface))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct FakeEnv(BTreeMap<String, OsString>);

    impl Env for FakeEnv {
        fn var_os(&self, name: &str) -> Option<OsString> {
            self.0.get(name).cloned()
        }
    }

    fn blank_checks() -> Vec<Check> {
        CHECK_IDS
            .into_iter()
            .map(|id| Check::skipped(id, "fixture"))
            .collect()
    }

    fn healthy_session() -> SessionHealth {
        use realm_core::ipc::{Capabilities, InterfaceVersion};
        SessionHealth {
            session_version: "0.1.0".to_owned(),
            protocol_version: PROTOCOL_VERSION,
            backend_name: "river".to_owned(),
            capabilities: Capabilities {
                exact_geometry: true,
                server_side_borders: true,
                hide_show: true,
                explicit_ordering: true,
                fullscreen: true,
                unsupported: Vec::new(),
            },
            bound_interfaces: [
                ("river_window_manager_v1", 4),
                ("river_xkb_bindings_v1", 3),
                ("river_layer_shell_v1", 1),
                ("river_input_manager_v1", 2),
                ("river_libinput_config_v1", 2),
            ]
            .into_iter()
            .map(|(name, version)| InterfaceVersion {
                name: name.to_owned(),
                version,
            })
            .collect(),
            layer_shell_served: true,
            degraded_codes: Some(Vec::new()),
            uptime_ms: 42,
        }
    }

    #[test]
    fn human_and_json_reports_preserve_the_exact_32_check_order() {
        let report = DoctorReport::fixture();
        assert_eq!(report.checks.len(), 32);
        assert_eq!(
            report
                .checks
                .iter()
                .map(|check| check.id)
                .collect::<Vec<_>>(),
            CHECK_IDS
        );

        let human = report.render_human();
        let json = report.render_json();
        let mut cursor = 0;
        for id in CHECK_IDS {
            let next = human[cursor..].find(id).expect("human report omitted id") + cursor;
            assert!(next >= cursor);
            cursor = next + id.len();
        }
        assert!(human.lines().all(|line| line.chars().count() <= 80));
        assert!(human.is_ascii());
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            value["checks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|check| check["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            CHECK_IDS
        );
    }

    #[test]
    fn session_entry_import_parser_matches_the_accepted_exact_list() {
        let fixture = r#"
SESSION_ENV_VARS=(
    WAYLAND_DISPLAY
    XDG_CURRENT_DESKTOP
    XDG_SESSION_TYPE
    XDG_SESSION_DESKTOP
    XDG_RUNTIME_DIR
    XCURSOR_THEME
    XCURSOR_SIZE
)
"#;
        assert_eq!(parse_session_imports(fixture).unwrap(), SESSION_ENV_VARS);
        assert_ne!(
            parse_session_imports(&fixture.replace("XCURSOR_SIZE", "DISPLAY")).unwrap(),
            SESSION_ENV_VARS
        );
    }

    #[test]
    fn session_entry_check_uses_the_wrapped_payload_not_the_launcher_shim() {
        let temp = tempfile::tempdir().unwrap();
        let shim = temp.path().join("realm-session");
        let payload = temp.path().join(".realm-session-wrapped");
        std::fs::write(&shim, "#!/bin/sh\nexec .realm-session-wrapped \"$@\"\n").unwrap();
        std::fs::write(
            &payload,
            "SESSION_ENV_VARS=(\nWAYLAND_DISPLAY\nXDG_CURRENT_DESKTOP\nXDG_SESSION_TYPE\nXDG_SESSION_DESKTOP\nXDG_RUNTIME_DIR\nXCURSOR_THEME\nXCURSOR_SIZE\n)\n",
        )
        .unwrap();

        let check = session_entry_import_check_from_installation(Some(temp.path()), []);
        assert_eq!(check.status, Status::Ok);
        assert!(check.summary.contains(".realm-session-wrapped"));
    }

    #[test]
    fn malformed_installed_payload_is_not_masked_by_a_valid_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let shim = temp.path().join("realm-session");
        let payload = temp.path().join(".realm-session-wrapped");
        let fallback = temp.path().join("source-realm-session");
        std::fs::write(&shim, "#!/bin/sh\nexec .realm-session-wrapped \"$@\"\n").unwrap();
        std::fs::write(&payload, "SESSION_ENV_VARS=(\nWAYLAND_DISPLAY\n)\n").unwrap();
        std::fs::write(
            &fallback,
            "SESSION_ENV_VARS=(\nWAYLAND_DISPLAY\nXDG_CURRENT_DESKTOP\nXDG_SESSION_TYPE\nXDG_SESSION_DESKTOP\nXDG_RUNTIME_DIR\nXCURSOR_THEME\nXCURSOR_SIZE\n)\n",
        )
        .unwrap();

        let check = session_entry_import_check_from_installation(Some(temp.path()), [fallback]);
        assert_eq!(check.status, Status::Fail);
        assert!(check.summary.contains(payload.to_str().unwrap()));
    }

    #[test]
    fn portal_failure_reports_an_observation_and_only_possible_causes() {
        let check = portal_proxy_check(
            true,
            true,
            &Err("portal proxy did not answer within 2000 ms".to_owned()),
            "env/wayland-display/dbus",
        );
        assert_eq!(check.status, Status::Fail);
        assert!(check.summary.contains("did not answer"));
        assert!(check.cause.as_deref().unwrap().contains("may be"));
        assert!(!check.summary.contains("DBUS_SESSION_BUS_ADDRESS="));
        assert!(!check.cause.as_deref().unwrap().contains("was missing"));
        assert!(check
            .remedy
            .as_deref()
            .unwrap()
            .contains("dbus-update-activation-environment --systemd"));
    }

    #[test]
    fn observed_tool_versions_never_claim_unresolved_floors_pass() {
        let complete = tool_floor_check([
            ("yazi", Ok("Yazi 25.5.31".to_owned())),
            ("btop", Ok("btop version: 1.4.0".to_owned())),
            ("starship", Ok("starship 1.23.0".to_owned())),
        ]);
        assert_eq!(complete.status, Status::Skip);
        assert!(complete.summary.contains("numeric floors unresolved"));

        let missing = tool_floor_check([
            ("yazi", Ok("Yazi 25.5.31".to_owned())),
            ("btop", Err("not found".to_owned())),
            ("starship", Ok("garbage".to_owned())),
        ]);
        assert_eq!(missing.status, Status::Warn);
        assert!(missing.summary.contains("btop: not found"));
        assert!(missing.summary.contains("starship: unparseable"));
    }

    #[test]
    fn missing_session_skips_health_while_dead_endpoint_fails_socket() {
        let mut absent = blank_checks();
        apply_session_checks(
            &mut absent,
            &SessionObservation::Absent("no realm session is running".to_owned()),
        );
        assert!(absent[..7].iter().all(|check| check.status == Status::Skip));

        let mut dead = blank_checks();
        apply_session_checks(
            &mut dead,
            &SessionObservation::Failed("connection refused".to_owned()),
        );
        assert_eq!(dead[0].status, Status::Fail);
        assert!(dead[1..7].iter().all(|check| check.status == Status::Skip));
    }

    #[test]
    fn live_health_populates_all_seven_session_and_backend_checks() {
        let mut checks = blank_checks();
        apply_session_checks(&mut checks, &SessionObservation::Live(healthy_session()));
        assert!(checks[..7].iter().all(|check| check.status == Status::Ok));
        assert!(checks[6].summary.contains("river_window_manager_v1 v4"));
    }

    #[test]
    fn portal_config_requires_routes_and_advertised_backend_interfaces() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config/xdg-desktop-portal");
        let portals = temp.path().join("data/xdg-desktop-portal/portals");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&portals).unwrap();
        std::fs::write(
            config.join("realm-portals.conf"),
            "[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.Settings=gtk\norg.freedesktop.impl.portal.Inhibit=none\norg.freedesktop.impl.portal.ScreenCast=wlr\norg.freedesktop.impl.portal.Screenshot=wlr\n",
        )
        .unwrap();
        std::fs::write(
            portals.join("gtk.portal"),
            "[portal]\nInterfaces=org.freedesktop.impl.portal.FileChooser;org.freedesktop.impl.portal.Settings;\n",
        )
        .unwrap();
        std::fs::write(
            portals.join("wlr.portal"),
            "[portal]\nInterfaces=org.freedesktop.impl.portal.ScreenCast;org.freedesktop.impl.portal.Screenshot;\n",
        )
        .unwrap();
        let env = FakeEnv(BTreeMap::from([
            (
                "XDG_CONFIG_HOME".to_owned(),
                config.parent().unwrap().as_os_str().to_owned(),
            ),
            (
                "XDG_DATA_DIRS".to_owned(),
                temp.path().join("data").as_os_str().to_owned(),
            ),
        ]));
        let mut checks = blank_checks();
        apply_portal_config_check(&mut checks, &env);
        assert_eq!(checks[24].status, Status::Ok);

        std::fs::write(
            portals.join("wlr.portal"),
            "[portal]\nInterfaces=org.freedesktop.impl.portal.Screenshot;\n",
        )
        .unwrap();
        apply_portal_config_check(&mut checks, &env);
        assert_eq!(checks[24].status, Status::Fail);
        assert!(checks[24].summary.contains(".portal metadata"));
    }

    #[test]
    fn restart_policy_check_matches_the_shipped_asymmetric_units() {
        let wm = include_str!("../../../packaging/systemd/realm-wm.service");
        let bar = include_str!("../../../packaging/systemd/realm-bar.service");
        assert!(restart_policy_valid(wm, bar));
        assert!(!restart_policy_valid(
            &wm.replace("Restart=always", "Restart=on-failure"),
            bar
        ));
    }

    #[test]
    fn external_probe_is_killed_at_its_deadline() {
        let start = Instant::now();
        let result = command_until(start + Duration::from_millis(40), "sleep", &["5"]);
        assert_eq!(result.unwrap_err(), "timed out");
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn portal_result_is_not_blocked_by_a_hung_systemd_probe() {
        let started = Instant::now();
        let receivers = begin_bus_observation_with(
            || -> Result<SystemdObservation, String> {
                std::thread::sleep(Duration::from_millis(200));
                Err("late systemd".to_owned())
            },
            || {
                Ok(PortalObservation {
                    filechooser_version: 7,
                    screencast_version: Ok(4),
                    filechooser_roundtrip: None,
                })
            },
        );
        let observed = finish_bus_observation(receivers, started + Duration::from_millis(40));
        assert!(observed.systemd.is_err());
        assert_eq!(observed.portal.unwrap().filechooser_version, 7);
        assert!(started.elapsed() < Duration::from_millis(150));
    }

    #[test]
    fn bus_deadline_is_absolute_from_probe_start() {
        let started = Instant::now();
        let receivers = begin_bus_observation_with(
            || -> Result<SystemdObservation, String> {
                std::thread::sleep(Duration::from_millis(200));
                Err("late systemd".to_owned())
            },
            || -> Result<PortalObservation, String> {
                std::thread::sleep(Duration::from_millis(200));
                Err("late portal".to_owned())
            },
        );
        std::thread::sleep(Duration::from_millis(25));
        let observed = finish_bus_observation(receivers, started + Duration::from_millis(50));
        assert!(observed.systemd.is_err());
        assert!(observed.portal.is_err());
        assert!(started.elapsed() < Duration::from_millis(150));
    }

    #[test]
    fn exited_probe_with_inherited_open_pipe_still_obeys_deadline() {
        let started = Instant::now();
        let result = command_until(
            started + Duration::from_millis(40),
            "/bin/sh",
            &["-c", "sleep 0.3 & printf ready"],
        );
        assert_eq!(result.unwrap_err(), "timed out reading command output");
        assert!(started.elapsed() < Duration::from_millis(150));
    }

    #[test]
    fn every_failing_check_has_symptom_cause_and_remedy() {
        let never_fail = [
            "session/degraded",
            "wm/attached",
            "wm/capabilities",
            "env/xwayland",
            "units/idle-lock",
            "theme/outputs",
            "fonts/glyphs",
            "fonts/attribution",
            "tools/floors",
        ];
        let mut checks = CHECK_IDS
            .into_iter()
            .map(|id| Check::new(id, Status::Fail, "fixture failure"))
            .collect::<Vec<_>>();
        ensure_failure_diagnostics(&mut checks);
        for check in checks
            .iter()
            .filter(|check| !never_fail.contains(&check.id))
        {
            assert!(check.symptom.is_some(), "{} symptom", check.id);
            assert!(check.cause.is_some(), "{} cause", check.id);
            assert!(check.remedy.is_some(), "{} remedy", check.id);
        }
    }
}
