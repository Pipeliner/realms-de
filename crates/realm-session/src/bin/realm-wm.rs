//! Production Realm window-manager and session-daemon entry point.

use realm_session::backend::{BackendError, RiverBackend};
use realm_session::consumer::{exec_from_env, FixedConsumer};
use realm_session::runtime::{run_production_daemon, RuntimeError};
use realm_session::session::SessionEventError;

fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if let [flag, owner] = args.as_slice() {
        if flag == "--prepare-session-theme" {
            if let Err(error) = prepare_login_theme(owner) {
                eprintln!("realm-wm: {error}");
                std::process::exit(1);
            }
            return;
        }
    }
    if !args.is_empty() {
        let consumer = match FixedConsumer::parse_args(&args) {
            Ok(consumer) => consumer,
            Err(error) => {
                eprintln!("realm-wm: {error}");
                std::process::exit(2);
            }
        };
        if let Err(error) = exec_from_env(consumer) {
            eprintln!("realm-wm: {error}");
            std::process::exit(1);
        }
        unreachable!("successful fixed-consumer exec does not return");
    }
    if let Err(error) = run_production_daemon(RiverBackend::from_env) {
        eprintln!("realm-wm: {error}");
        std::process::exit(exit_status(&error));
    }
}

fn prepare_login_theme(owner: &std::ffi::OsStr) -> Result<(), String> {
    let owner = owner
        .to_str()
        .ok_or("login owner PID is not UTF-8")?
        .parse::<u32>()
        .map_err(|_| "login owner PID must be a positive integer")?;
    let root = realm_session::consumer::config_root_from_env()?;
    let runtime = realm_control::production_runtime_dir().map_err(|error| error.to_string())?;
    realm_session::login_theme::prepare(&root, runtime.path(), owner)
}

fn exit_status(error: &RuntimeError) -> i32 {
    let backend = match error {
        RuntimeError::Backend(error) => Some(error),
        RuntimeError::Session(SessionEventError::Backend(error)) => Some(error),
        _ => None,
    };
    match backend {
        Some(BackendError::Unavailable { message }) if message.starts_with("required global ") => {
            78
        }
        Some(BackendError::Unavailable { message })
            if message == "another River window manager is active" =>
        {
            69
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use realm_session::backend::BackendError;
    use realm_session::runtime::RuntimeError;

    use super::exit_status;

    #[test]
    fn permanent_river_refusals_have_the_service_contract_exit_codes() {
        assert_eq!(
            exit_status(&RuntimeError::Backend(BackendError::Unavailable {
                message: "another River window manager is active".to_owned(),
            })),
            69
        );
        assert_eq!(
            exit_status(&RuntimeError::Backend(BackendError::Unavailable {
                message: "required global river_window_manager_v1 advertised version 3, required 4"
                    .to_owned(),
            })),
            78
        );
        assert_eq!(
            exit_status(&RuntimeError::Backend(BackendError::Unavailable {
                message: "selected River seat was removed".to_owned(),
            })),
            1
        );
    }
}
