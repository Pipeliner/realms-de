//! Linux timerfd owners used by the session poll loop.

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustix::event::Timespec;
use rustix::io::Errno;
use rustix::time::{
    timerfd_create, timerfd_settime, Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags,
};

use crate::session::RepeatTimerDirective;

/// The clock and held-key timers owned by the event-loop thread.
pub struct SessionTimers {
    clock: OwnedFd,
    repeat: OwnedFd,
}

impl SessionTimers {
    /// Create nonblocking close-on-exec timers and arm the next clock boundary.
    pub fn new() -> io::Result<Self> {
        let flags = TimerfdFlags::CLOEXEC | TimerfdFlags::NONBLOCK;
        let timers = Self {
            clock: timerfd_create(TimerfdClockId::Monotonic, flags)?,
            repeat: timerfd_create(TimerfdClockId::Monotonic, flags)?,
        };
        timers.arm_next_clock(SystemTime::now())?;
        Ok(timers)
    }

    /// Borrow the minute-boundary clock descriptor.
    pub fn clock_fd(&self) -> BorrowedFd<'_> {
        self.clock.as_fd()
    }

    /// Borrow the held-key repeat descriptor.
    pub fn repeat_fd(&self) -> BorrowedFd<'_> {
        self.repeat.as_fd()
    }

    /// Apply the immediate timer edge emitted by Session.
    pub fn apply_repeat(&self, directive: &RepeatTimerDirective) -> io::Result<()> {
        let timer = match directive {
            RepeatTimerDirective::Preserve => return Ok(()),
            RepeatTimerDirective::Arm { delay, interval } => Itimerspec {
                it_interval: timespec(*interval)?,
                it_value: timespec(*delay)?,
            },
            RepeatTimerDirective::Disarm => Itimerspec {
                it_interval: Timespec::default(),
                it_value: Timespec::default(),
            },
        };
        timerfd_settime(&self.repeat, TimerfdTimerFlags::empty(), &timer)?;
        Ok(())
    }

    /// Consume all accumulated repeat expirations as one bounded wakeup.
    pub fn consume_repeat(&self) -> io::Result<bool> {
        consume(&self.repeat)
    }

    /// Consume a clock wakeup and schedule the next minute boundary.
    pub fn consume_clock(&self, now: SystemTime) -> io::Result<bool> {
        let expired = consume(&self.clock)?;
        if expired {
            self.arm_next_clock(now)?;
        }
        Ok(expired)
    }

    fn arm_next_clock(&self, now: SystemTime) -> io::Result<()> {
        let since_epoch = now.duration_since(UNIX_EPOCH).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "system clock precedes Unix epoch",
            )
        })?;
        let timer = Itimerspec {
            it_interval: Timespec::default(),
            it_value: timespec(duration_until_next_minute(since_epoch))?,
        };
        timerfd_settime(&self.clock, TimerfdTimerFlags::empty(), &timer)?;
        Ok(())
    }
}

fn consume(fd: &OwnedFd) -> io::Result<bool> {
    let mut bytes = [0_u8; 8];
    match rustix::io::read(fd, &mut bytes) {
        Ok(8) => Ok(u64::from_ne_bytes(bytes) > 0),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "timerfd returned a partial counter",
        )),
        Err(Errno::AGAIN) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn timespec(duration: Duration) -> io::Result<Timespec> {
    duration.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "timer duration does not fit timespec",
        )
    })
}

fn duration_until_next_minute(since_epoch: Duration) -> Duration {
    let nanos_per_minute = 60_u128 * 1_000_000_000;
    let elapsed = since_epoch.as_nanos() % nanos_per_minute;
    let remaining = nanos_per_minute - elapsed;
    Duration::new(
        (remaining / 1_000_000_000) as u64,
        (remaining % 1_000_000_000) as u32,
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rustix::time::timerfd_gettime;

    use super::SessionTimers;
    use crate::session::RepeatTimerDirective;

    #[test]
    fn repeat_timer_directives_program_single_timerfd_exactly() {
        let timers = SessionTimers::new().unwrap();
        timers
            .apply_repeat(&RepeatTimerDirective::Arm {
                delay: Duration::from_millis(600),
                interval: Duration::from_millis(40),
            })
            .unwrap();
        let armed = timerfd_gettime(timers.repeat_fd()).unwrap();
        assert_eq!(armed.it_interval.tv_sec, 0);
        assert_eq!(armed.it_interval.tv_nsec, 40_000_000);
        assert!(armed.it_value.tv_sec == 0 && armed.it_value.tv_nsec > 0);
        assert!(armed.it_value.tv_nsec <= 600_000_000);

        timers.apply_repeat(&RepeatTimerDirective::Disarm).unwrap();
        let disarmed = timerfd_gettime(timers.repeat_fd()).unwrap();
        assert_eq!(disarmed.it_value.tv_sec, 0);
        assert_eq!(disarmed.it_value.tv_nsec, 0);
    }

    #[test]
    fn duration_until_next_minute_is_nonzero_and_boundary_aligned() {
        assert_eq!(
            super::duration_until_next_minute(Duration::from_secs(61)),
            Duration::from_secs(59)
        );
        assert_eq!(
            super::duration_until_next_minute(Duration::from_secs(120)),
            Duration::from_secs(60)
        );
    }
}
