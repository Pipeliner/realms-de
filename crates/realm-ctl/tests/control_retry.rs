#[path = "../src/retry.rs"]
mod retry;

use std::{
    cell::{Cell, RefCell},
    process::ExitCode,
    time::{Duration, Instant},
};

use realm_control::ClientError;

struct FakeClock {
    start: Instant,
    now: Cell<Instant>,
    attempts: RefCell<Vec<Duration>>,
    sleeps: RefCell<Vec<Duration>>,
}

impl FakeClock {
    fn new(start: Instant) -> Self {
        Self {
            start,
            now: Cell::new(start),
            attempts: RefCell::new(Vec::new()),
            sleeps: RefCell::new(Vec::new()),
        }
    }

    fn now(&self) -> Instant {
        self.now.get()
    }

    fn record_attempt(&self) {
        self.attempts
            .borrow_mut()
            .push(self.now().duration_since(self.start));
    }

    fn sleep(&self, duration: Duration) {
        assert!(!duration.is_zero(), "retry driver requested a zero sleep");
        self.sleeps.borrow_mut().push(duration);
        self.advance(duration);
    }

    fn advance(&self, duration: Duration) {
        self.now.set(self.now() + duration);
    }
}

#[test]
fn immediate_retryable_failures_use_exact_absolute_schedule_without_a_final_sleep() {
    let start = Instant::now();
    let clock = FakeClock::new(start);

    let result = retry::run(
        start,
        || clock.now(),
        |duration| clock.sleep(duration),
        || {
            clock.record_attempt();
            Err(ClientError::MissingRealm)
        },
    );

    assert!(matches!(result, Err(ClientError::MissingRealm)));
    assert_eq!(
        *clock.attempts.borrow(),
        vec![
            Duration::from_millis(0),
            Duration::from_millis(10),
            Duration::from_millis(30),
            Duration::from_millis(70),
            Duration::from_millis(150),
            Duration::from_millis(310),
        ]
    );
    assert_eq!(
        *clock.sleeps.borrow(),
        vec![
            Duration::from_millis(10),
            Duration::from_millis(20),
            Duration::from_millis(40),
            Duration::from_millis(80),
            Duration::from_millis(160),
        ]
    );
}

#[test]
fn late_retryable_attempts_start_immediately_when_their_targets_have_elapsed() {
    let start = Instant::now();
    let clock = FakeClock::new(start);
    let attempt_number = Cell::new(0);

    let result = retry::run(
        start,
        || clock.now(),
        |duration| clock.sleep(duration),
        || {
            clock.record_attempt();
            let number = attempt_number.get();
            attempt_number.set(number + 1);
            if number == 0 {
                clock.advance(Duration::from_millis(90));
            }
            Err(ClientError::Refused)
        },
    );

    assert!(matches!(result, Err(ClientError::Refused)));
    assert_eq!(
        *clock.attempts.borrow(),
        vec![
            Duration::from_millis(0),
            Duration::from_millis(90),
            Duration::from_millis(90),
            Duration::from_millis(90),
            Duration::from_millis(150),
            Duration::from_millis(310),
        ]
    );
    assert_eq!(
        *clock.sleeps.borrow(),
        vec![Duration::from_millis(60), Duration::from_millis(160)]
    );
}

#[test]
fn terminal_error_stops_after_its_first_attempt() {
    let start = Instant::now();
    let clock = FakeClock::new(start);

    let result = retry::run(
        start,
        || clock.now(),
        |duration| clock.sleep(duration),
        || {
            clock.record_attempt();
            Err(ClientError::VersionMismatch {
                client: 1,
                server: 2,
            })
        },
    );

    assert!(matches!(result, Err(ClientError::VersionMismatch { .. })));
    assert_eq!(*clock.attempts.borrow(), vec![Duration::ZERO]);
    assert!(clock.sleeps.borrow().is_empty());
}

#[test]
fn classifier_uses_the_specified_client_exit_codes() {
    assert_eq!(
        retry::classify_exit(&ClientError::MissingRealm),
        ExitCode::from(3)
    );
    assert_eq!(
        retry::classify_exit(&ClientError::Refused),
        ExitCode::from(3)
    );
    assert_eq!(
        retry::classify_exit(&ClientError::VersionMismatch {
            client: 1,
            server: 2,
        }),
        ExitCode::from(4)
    );
    assert_eq!(
        retry::classify_exit(&ClientError::InvalidRequest),
        ExitCode::from(6)
    );
}
