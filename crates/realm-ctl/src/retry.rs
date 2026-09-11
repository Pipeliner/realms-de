use std::{
    process::ExitCode,
    time::{Duration, Instant},
};

use realm_control::{Client, ClientError};

const STARTUP_OFFSETS: [Duration; 6] = [
    Duration::from_millis(0),
    Duration::from_millis(10),
    Duration::from_millis(30),
    Duration::from_millis(70),
    Duration::from_millis(150),
    Duration::from_millis(310),
];

pub(crate) fn run<Now, Sleep, Attempt>(
    start: Instant,
    mut now: Now,
    mut sleep: Sleep,
    mut attempt: Attempt,
) -> Result<Client, ClientError>
where
    Now: FnMut() -> Instant,
    Sleep: FnMut(Duration),
    Attempt: FnMut() -> Result<Client, ClientError>,
{
    for (index, offset) in STARTUP_OFFSETS.into_iter().enumerate() {
        if index != 0 {
            let target = start + offset;
            let current = now();
            if current < target {
                sleep(target.duration_since(current));
            }
        }

        match attempt() {
            Ok(client) => return Ok(client),
            Err(error) if error.is_retryable() && index + 1 < STARTUP_OFFSETS.len() => {}
            Err(error) => return Err(error),
        }
    }

    unreachable!("the final startup retry attempt always returns")
}

pub(crate) fn classify_exit(error: &ClientError) -> ExitCode {
    match error {
        ClientError::MissingRealm | ClientError::Refused => ExitCode::from(3),
        ClientError::VersionMismatch { .. } => ExitCode::from(4),
        _ => ExitCode::from(6),
    }
}
