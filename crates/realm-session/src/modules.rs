//! Runtime-owned bar module producers.

use std::io;
use std::time::SystemTime;

use jiff::tz::TimeZone;
use jiff::Timestamp;
use realm_core::state::Module;

/// Minute-granular local wall-clock producer with timezone state loaded at startup.
pub struct ClockModule {
    timezone: TimeZone,
}

impl ClockModule {
    /// Capture the system timezone before the live event loop begins.
    pub fn system() -> Self {
        Self {
            timezone: TimeZone::system(),
        }
    }

    #[cfg(test)]
    fn utc() -> Self {
        Self {
            timezone: TimeZone::UTC,
        }
    }

    /// Render one fixed-width `HH:MM` module at the supplied wall-clock instant.
    pub fn render(&self, now: SystemTime) -> io::Result<Module> {
        let timestamp = Timestamp::try_from(now)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let zoned = timestamp.to_zoned(self.timezone.clone());
        Ok(Module {
            id: "clock".to_owned(),
            text: zoned.strftime("%H:%M").to_string(),
            accent: None,
            urgent: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::ClockModule;

    #[test]
    fn clock_is_fixed_width_and_uses_the_captured_timezone() {
        let clock = ClockModule::utc();
        let midnight = clock
            .render(UNIX_EPOCH + Duration::from_secs(1_735_689_600))
            .unwrap();
        let afternoon = clock
            .render(UNIX_EPOCH + Duration::from_secs(1_735_689_600 + 13 * 60 * 60 + 7 * 60))
            .unwrap();

        assert_eq!(midnight.id, "clock");
        assert_eq!(midnight.text, "00:00");
        assert_eq!(afternoon.text, "13:07");
        assert_eq!(midnight.text.len(), afternoon.text.len());
        assert_eq!(midnight.accent, None);
        assert!(!midnight.urgent);
    }
}
