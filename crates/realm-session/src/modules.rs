//! Runtime-owned bar module producers.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{Duration, Instant, SystemTime};

use jiff::tz::TimeZone;
use jiff::Timestamp;
use nix::sys::socket::{
    bind, recv, socket, AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType,
};
use realm_core::state::Module;
use rustix::event::{eventfd, poll, EventfdFlags, PollFd, PollFlags};
use rustix::io::Errno;

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
    pub(crate) fn utc() -> Self {
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

/// One numeric network observation ready for exact presentation formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkRate {
    /// Aggregate receive bytes per second across comparable non-loopback interfaces.
    pub receive_bps: u64,
    /// Aggregate transmit bytes per second across comparable non-loopback interfaces.
    pub transmit_bps: u64,
}

/// One selected battery observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryReading {
    capacity: u8,
    discharging: bool,
}

impl BatteryReading {
    fn new(capacity: u8, discharging: bool) -> Self {
        Self {
            capacity,
            discharging,
        }
    }

    fn module(self) -> Module {
        Module {
            id: "battery".to_owned(),
            text: format!("⚡ {}%", self.capacity),
            accent: Some("gold".to_owned()),
            urgent: self.discharging && self.capacity <= 15,
        }
    }
}

/// Fixed-size numeric values handed from the sampler to the event-loop owner.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModuleSnapshot {
    /// Rounded aggregate CPU busy percent.
    pub cpu_percent: Option<u8>,
    /// Used memory in tenths of a GiB.
    pub memory_tenths_gib: Option<u64>,
    /// Aggregate non-loopback network rates.
    pub network: Option<NetworkRate>,
    /// Selected battery, absent when no battery is present or observation is disabled.
    pub battery: Option<BatteryReading>,
}

impl ModuleSnapshot {
    /// Format the exact ordered MVP module vector and append the supplied clock.
    pub fn modules(self, clock: Module) -> Vec<Module> {
        let mut modules = Vec::with_capacity(5);
        if let Some(network) = self.network {
            modules.push(Module {
                id: "net".to_owned(),
                text: format!(
                    "↑ {} ↓ {}",
                    format_rate(network.transmit_bps),
                    format_rate(network.receive_bps)
                ),
                accent: None,
                urgent: false,
            });
        }
        if let Some(percent) = self.cpu_percent {
            modules.push(Module {
                id: "cpu".to_owned(),
                text: format!("cpu {percent}%"),
                accent: Some("starlight".to_owned()),
                urgent: false,
            });
        }
        if let Some(tenths) = self.memory_tenths_gib {
            modules.push(Module {
                id: "mem".to_owned(),
                text: format!("mem {}.{}G", tenths / 10, tenths % 10),
                accent: None,
                urgent: false,
            });
        }
        if let Some(battery) = self.battery {
            modules.push(battery.module());
        }
        modules.push(clock);
        modules
    }
}

fn format_rate(bytes_per_second: u64) -> String {
    const UNITS: &[(u64, &str)] = &[(1_000_000_000, "G"), (1_000_000, "M"), (1_000, "k")];
    for &(scale, suffix) in UNITS {
        if bytes_per_second >= scale {
            let tenths = ((bytes_per_second as u128 * 10 + u128::from(scale / 2))
                / u128::from(scale)) as u64;
            return if tenths.is_multiple_of(10) {
                format!("{}{suffix}", tenths / 10)
            } else {
                format!("{}.{}{suffix}", tenths / 10, tenths % 10)
            };
        }
    }
    bytes_per_second.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Source {
    Cpu,
    Memory,
    Network,
}

#[derive(Debug, Clone, Copy)]
struct CpuCounters {
    total: u64,
    idle: u64,
}

#[derive(Debug, Clone, Copy)]
struct NetCounters {
    receive: u64,
    transmit: u64,
}

#[derive(Debug)]
struct ProcObservation {
    snapshot: ModuleSnapshot,
    new_failures: Vec<Source>,
}

/// Stateful reducer for checked `/proc` observations.
#[derive(Default)]
struct ProcSampler {
    previous_cpu: Option<CpuCounters>,
    previous_network: BTreeMap<String, NetCounters>,
    cpu_percent: Option<u8>,
    memory_tenths_gib: Option<u64>,
    network: Option<NetworkRate>,
    failures: HashSet<Source>,
}

impl ProcSampler {
    fn observe(
        &mut self,
        stat: &str,
        meminfo: &str,
        netdev: &str,
        elapsed: Duration,
    ) -> ProcObservation {
        let mut new_failures = Vec::new();
        match parse_cpu(stat) {
            Ok(current) => {
                self.clear_failure(Source::Cpu);
                if let Some(previous) = self.previous_cpu {
                    if let (Some(total), Some(idle)) = (
                        current.total.checked_sub(previous.total),
                        current.idle.checked_sub(previous.idle),
                    ) {
                        if total > 0 && idle <= total {
                            let busy = total - idle;
                            let rounded = (u128::from(busy) * 100 + u128::from(total / 2))
                                / u128::from(total);
                            self.cpu_percent = Some(rounded.min(100) as u8);
                        }
                    }
                }
                self.previous_cpu = Some(current);
            }
            Err(()) => {
                self.previous_cpu = None;
                self.note_failure(Source::Cpu, &mut new_failures);
            }
        }
        match parse_memory(meminfo) {
            Ok(value) => {
                self.clear_failure(Source::Memory);
                self.memory_tenths_gib = Some(value);
            }
            Err(()) => self.note_failure(Source::Memory, &mut new_failures),
        }
        match parse_network(netdev) {
            Ok(current) => {
                let mut calculation_failed = false;
                if !elapsed.is_zero() {
                    let mut receive = 0_u64;
                    let mut transmit = 0_u64;
                    let mut comparable = false;
                    let mut valid = true;
                    for (name, counters) in &current {
                        let Some(previous) = self.previous_network.get(name) else {
                            continue;
                        };
                        let (Some(rx), Some(tx)) = (
                            counters.receive.checked_sub(previous.receive),
                            counters.transmit.checked_sub(previous.transmit),
                        ) else {
                            continue;
                        };
                        comparable = true;
                        let Some(next_receive) = receive.checked_add(rx) else {
                            valid = false;
                            break;
                        };
                        let Some(next_transmit) = transmit.checked_add(tx) else {
                            valid = false;
                            break;
                        };
                        receive = next_receive;
                        transmit = next_transmit;
                    }
                    if comparable && valid {
                        if let (Some(receive_bps), Some(transmit_bps)) = (
                            rate_per_second(receive, elapsed),
                            rate_per_second(transmit, elapsed),
                        ) {
                            self.network = Some(NetworkRate {
                                receive_bps,
                                transmit_bps,
                            });
                        } else {
                            calculation_failed = true;
                        }
                    } else if !valid {
                        calculation_failed = true;
                    }
                }
                self.previous_network = current;
                if calculation_failed {
                    self.note_failure(Source::Network, &mut new_failures);
                } else {
                    self.clear_failure(Source::Network);
                }
            }
            Err(()) => {
                self.previous_network.clear();
                self.note_failure(Source::Network, &mut new_failures);
            }
        }
        ProcObservation {
            snapshot: self.snapshot(),
            new_failures,
        }
    }

    fn snapshot(&self) -> ModuleSnapshot {
        ModuleSnapshot {
            cpu_percent: self.cpu_percent,
            memory_tenths_gib: self.memory_tenths_gib,
            network: self.network,
            battery: None,
        }
    }

    fn note_failure(&mut self, source: Source, new_failures: &mut Vec<Source>) {
        if self.failures.insert(source) {
            new_failures.push(source);
        }
    }

    fn clear_failure(&mut self, source: Source) {
        self.failures.remove(&source);
    }
}

fn parse_cpu(input: &str) -> Result<CpuCounters, ()> {
    let line = input
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or(())?;
    let values = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ())?;
    if values.len() != 8 {
        return Err(());
    }
    let total = values
        .iter()
        .try_fold(0_u64, |sum, value| sum.checked_add(*value))
        .ok_or(())?;
    let idle = values[3].checked_add(values[4]).ok_or(())?;
    Ok(CpuCounters { total, idle })
}

fn parse_memory(input: &str) -> Result<u64, ()> {
    let mut total = None;
    let mut available = None;
    for line in input.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else { continue };
        if name != "MemTotal:" && name != "MemAvailable:" {
            continue;
        }
        let value = fields.next().ok_or(())?.parse::<u64>().map_err(|_| ())?;
        if fields.next() != Some("kB") {
            return Err(());
        }
        match name {
            "MemTotal:" => total = Some(value),
            "MemAvailable:" => available = Some(value),
            _ => unreachable!(),
        }
    }
    let used = total
        .ok_or(())?
        .checked_sub(available.ok_or(())?)
        .ok_or(())?;
    let tenths = (u128::from(used) * 10 + 524_288) / 1_048_576;
    u64::try_from(tenths).map_err(|_| ())
}

fn parse_network(input: &str) -> Result<BTreeMap<String, NetCounters>, ()> {
    let mut counters = BTreeMap::new();
    let mut saw_interface = false;
    for line in input.lines() {
        let Some((name, fields)) = line.split_once(':') else {
            continue;
        };
        saw_interface = true;
        let name = name.trim();
        let values = fields
            .split_whitespace()
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ())?;
        if values.len() < 16 {
            return Err(());
        }
        if name != "lo" {
            counters.insert(
                name.to_owned(),
                NetCounters {
                    receive: values[0],
                    transmit: values[8],
                },
            );
        }
    }
    saw_interface.then_some(counters).ok_or(())
}

fn rate_per_second(bytes: u64, elapsed: Duration) -> Option<u64> {
    let nanos = elapsed.as_nanos();
    if nanos == 0 {
        return None;
    }
    let rounded = (u128::from(bytes).checked_mul(1_000_000_000)? + nanos / 2) / nanos;
    u64::try_from(rounded).ok()
}

fn scan_battery(root: &Path) -> io::Result<Option<BatteryReading>> {
    let mut entries = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if fs::read_to_string(path.join("type"))?.trim() != "Battery" {
            continue;
        }
        let capacity = fs::read_to_string(path.join("capacity"))?
            .trim()
            .parse::<u8>()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if capacity > 100 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "battery capacity exceeds 100",
            ));
        }
        let discharging = fs::read_to_string(path.join("status"))?.trim() == "Discharging";
        return Ok(Some(BatteryReading::new(capacity, discharging)));
    }
    Ok(None)
}

#[derive(Default)]
struct BatterySampler {
    reading: Option<BatteryReading>,
    failed: bool,
}

impl BatterySampler {
    fn observe(&mut self, result: io::Result<Option<BatteryReading>>) -> bool {
        match result {
            Ok(reading) => {
                self.reading = reading;
                self.failed = false;
                false
            }
            Err(_) if !self.failed => {
                self.failed = true;
                true
            }
            Err(_) => false,
        }
    }

    fn reading(&self) -> Option<BatteryReading> {
        self.reading
    }
}

fn is_power_supply_uevent(message: &[u8]) -> bool {
    message
        .split(|byte| *byte == 0)
        .any(|field| field == b"SUBSYSTEM=power_supply")
}

struct SamplerMailbox {
    slot: Arc<Mutex<Option<ModuleSnapshot>>>,
    event_fd: Arc<OwnedFd>,
}

impl SamplerMailbox {
    fn new() -> io::Result<Self> {
        Ok(Self {
            slot: Arc::new(Mutex::new(None)),
            event_fd: Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?),
        })
    }

    fn publish(&self, snapshot: ModuleSnapshot) -> io::Result<()> {
        *self
            .slot
            .lock()
            .map_err(|_| io::Error::other("module sampler latest-value mutex was poisoned"))? =
            Some(snapshot);
        match rustix::io::write(&self.event_fd, &1_u64.to_ne_bytes()) {
            Ok(8) | Err(Errno::AGAIN) => Ok(()),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "module sampler eventfd accepted a partial counter",
            )),
            Err(error) => Err(io::Error::from(error)),
        }
    }

    fn try_take_latest(&self) -> io::Result<Option<ModuleSnapshot>> {
        let mut slot = match self.slot.try_lock() {
            Ok(slot) => slot,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => {
                return Err(io::Error::other(
                    "module sampler latest-value mutex was poisoned",
                ))
            }
        };
        drain_eventfd(self.event_fd.as_fd())?;
        Ok(slot.take())
    }
}

/// Runtime handle for the single off-input-path system-module sampler.
pub struct ModuleSampler {
    mailbox: SamplerMailbox,
    stop_fd: Arc<OwnedFd>,
    stopped: bool,
}

impl ModuleSampler {
    /// Start the sampler without waiting for its first observation.
    pub fn start() -> io::Result<Self> {
        let mailbox = SamplerMailbox::new()?;
        let stop_fd = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?);
        let thread_mailbox = SamplerMailbox {
            slot: mailbox.slot.clone(),
            event_fd: mailbox.event_fd.clone(),
        };
        let thread_stop = stop_fd.clone();
        std::thread::Builder::new()
            .name("realm-modules".to_owned())
            .spawn(move || sampler_main(thread_mailbox, thread_stop))?;
        Ok(Self {
            mailbox,
            stop_fd,
            stopped: false,
        })
    }

    /// Borrow the latest-value notification descriptor.
    pub fn event_fd(&self) -> BorrowedFd<'_> {
        self.mailbox.event_fd.as_fd()
    }

    /// Attempt one nonblocking take; contention deliberately leaves readiness pending.
    pub fn try_take_latest(&self) -> io::Result<Option<ModuleSnapshot>> {
        self.mailbox.try_take_latest()
    }

    /// Signal terminal shutdown without joining the sampler thread.
    pub fn stop(&mut self) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        match rustix::io::write(&self.stop_fd, &1_u64.to_ne_bytes()) {
            Ok(8) | Err(Errno::AGAIN) => Ok(()),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "module sampler stop eventfd accepted a partial counter",
            )),
            Err(error) => Err(io::Error::from(error)),
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture() -> io::Result<Self> {
        Ok(Self {
            mailbox: SamplerMailbox::new()?,
            stop_fd: Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?),
            stopped: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn publish_fixture(&self, snapshot: ModuleSnapshot) -> io::Result<()> {
        self.mailbox.publish(snapshot)
    }

    #[cfg(test)]
    pub(crate) fn stopped(&self) -> bool {
        self.stopped
    }
}

impl Drop for ModuleSampler {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn sampler_main(mailbox: SamplerMailbox, stop_fd: Arc<OwnedFd>) {
    let mut proc = ProcSampler::default();
    let mut battery = BatterySampler::default();
    let mut uevent = match open_power_uevent() {
        Ok(fd) => Some(fd),
        Err(error) => {
            eprintln!("realm-wm: battery observation disabled: {error}");
            None
        }
    };
    let paths = SamplerPaths::system();
    let mut last_sample = Instant::now();
    sample_proc_and_publish(&mut proc, &paths, Duration::ZERO, &mailbox, &battery);
    if uevent.is_some() {
        sample_battery_and_publish(&proc, &mut battery, &paths, &mailbox);
    }
    let mut next_sample = Instant::now() + Duration::from_secs(1);
    loop {
        let mut fds = vec![PollFd::from_borrowed_fd(stop_fd.as_fd(), PollFlags::IN)];
        if let Some(uevent) = &uevent {
            fds.push(PollFd::from_borrowed_fd(uevent.as_fd(), PollFlags::IN));
        }
        let timeout: rustix::event::Timespec = next_sample
            .saturating_duration_since(Instant::now())
            .try_into()
            .unwrap_or_default();
        match poll(&mut fds, Some(&timeout)) {
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(error) => {
                eprintln!("realm-wm: module sampler poll failed: {error}");
                return;
            }
        }
        if fds[0]
            .revents()
            .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP)
        {
            return;
        }
        let uevent_events = fds
            .get(1)
            .map(PollFd::revents)
            .unwrap_or_else(PollFlags::empty);
        drop(fds);
        let battery_changed =
            if uevent_events.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) {
                eprintln!("realm-wm: battery uevent source became unavailable");
                uevent = None;
                false
            } else if uevent_events.contains(PollFlags::IN) {
                match drain_power_uevents(uevent.as_ref().expect("checked uevent presence")) {
                    Ok(changed) => changed,
                    Err(error) => {
                        eprintln!("realm-wm: battery uevent read failed: {error}");
                        uevent = None;
                        false
                    }
                }
            } else {
                false
            };
        let now = Instant::now();
        let interval_due = now >= next_sample;
        if interval_due {
            let elapsed = now.saturating_duration_since(last_sample);
            sample_proc_and_publish(&mut proc, &paths, elapsed, &mailbox, &battery);
            last_sample = now;
            next_sample = now + Duration::from_secs(1);
        }
        if battery_changed {
            sample_battery_and_publish(&proc, &mut battery, &paths, &mailbox);
        }
    }
}

struct SamplerPaths {
    stat: PathBuf,
    meminfo: PathBuf,
    netdev: PathBuf,
    power_supply: PathBuf,
}

impl SamplerPaths {
    fn system() -> Self {
        Self {
            stat: PathBuf::from("/proc/stat"),
            meminfo: PathBuf::from("/proc/meminfo"),
            netdev: PathBuf::from("/proc/net/dev"),
            power_supply: PathBuf::from("/sys/class/power_supply"),
        }
    }
}

fn sample_proc_and_publish(
    proc: &mut ProcSampler,
    paths: &SamplerPaths,
    elapsed: Duration,
    mailbox: &SamplerMailbox,
    battery: &BatterySampler,
) {
    let stat = fs::read_to_string(&paths.stat);
    let meminfo = fs::read_to_string(&paths.meminfo);
    let netdev = fs::read_to_string(&paths.netdev);
    let observation = proc.observe(
        stat.as_deref().unwrap_or(""),
        meminfo.as_deref().unwrap_or(""),
        netdev.as_deref().unwrap_or(""),
        elapsed,
    );
    for source in observation.new_failures {
        eprintln!("realm-wm: {source:?} module source observation failed");
    }
    let mut snapshot = observation.snapshot;
    snapshot.battery = battery.reading();
    if let Err(error) = mailbox.publish(snapshot) {
        eprintln!("realm-wm: module sampler publication failed: {error}");
    }
}

fn sample_battery_and_publish(
    proc: &ProcSampler,
    battery: &mut BatterySampler,
    paths: &SamplerPaths,
    mailbox: &SamplerMailbox,
) {
    let result = scan_battery(&paths.power_supply);
    let error = result.as_ref().err().map(ToString::to_string);
    if battery.observe(result) {
        eprintln!(
            "realm-wm: battery module source observation failed: {}",
            error.expect("a new failure has an error")
        );
    }
    let mut snapshot = proc.snapshot();
    snapshot.battery = battery.reading();
    if let Err(error) = mailbox.publish(snapshot) {
        eprintln!("realm-wm: module sampler publication failed: {error}");
    }
}

fn open_power_uevent() -> io::Result<OwnedFd> {
    let socket = socket(
        AddressFamily::Netlink,
        SockType::Datagram,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
        SockProtocol::NetlinkKObjectUEvent,
    )?;
    bind(socket.as_raw_fd(), &NetlinkAddr::new(0, 1))?;
    Ok(socket)
}

fn drain_power_uevents(fd: &OwnedFd) -> nix::Result<bool> {
    let mut power_supply = false;
    loop {
        let mut buffer = [0_u8; 4096];
        match recv(fd.as_raw_fd(), &mut buffer, MsgFlags::MSG_DONTWAIT) {
            Ok(length) => power_supply |= is_power_supply_uevent(&buffer[..length]),
            Err(nix::errno::Errno::EAGAIN) => return Ok(power_supply),
            Err(error) => return Err(error),
        }
    }
}

fn drain_eventfd(fd: BorrowedFd<'_>) -> io::Result<()> {
    let mut bytes = [0_u8; 8];
    match rustix::io::read(fd, &mut bytes) {
        Ok(8) | Err(Errno::AGAIN) => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "module sampler eventfd returned a partial counter",
        )),
        Err(error) => Err(io::Error::from(error)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::os::fd::AsFd;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use rustix::event::{poll, PollFd, PollFlags};

    use super::{
        scan_battery, BatteryReading, BatterySampler, ClockModule, ModuleSnapshot, ProcSampler,
        SamplerMailbox, Source,
    };

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

    #[test]
    fn first_and_second_proc_samples_format_the_exact_mvp_modules() {
        let mut sampler = ProcSampler::default();
        let stat_1 = "cpu 100 20 30 850 10 5 5 10 99 88\n";
        let stat_2 = "cpu 120 20 40 910 20 5 5 10 100 90\n";
        let meminfo = "MemTotal:       16777216 kB\nMemAvailable:    6291456 kB\n";
        let net_1 = "Inter-| Receive | Transmit\n lo: 9000 0 0 0 0 0 0 0 9000 0 0 0 0 0 0 0\n eth0: 1000 0 0 0 0 0 0 0 2000 0 0 0 0 0 0 0\n";
        let net_2 = "Inter-| Receive | Transmit\n lo: 999999 0 0 0 0 0 0 0 999999 0 0 0 0 0 0 0\n eth0: 1501000 0 0 0 0 0 0 0 752000 0 0 0 0 0 0 0\n";

        let first = sampler.observe(stat_1, meminfo, net_1, Duration::ZERO);
        assert_eq!(first.snapshot.cpu_percent, None);
        assert_eq!(first.snapshot.network, None);
        assert_eq!(first.snapshot.memory_tenths_gib, Some(100));

        let second = sampler.observe(stat_2, meminfo, net_2, Duration::from_secs(2));
        let clock = ClockModule::utc()
            .render(UNIX_EPOCH + Duration::from_secs(1_735_689_600))
            .unwrap();
        assert_eq!(
            second.snapshot.modules(clock),
            vec![
                realm_core::state::Module {
                    id: "net".to_owned(),
                    text: "↑ 375k ↓ 750k".to_owned(),
                    accent: None,
                    urgent: false,
                },
                realm_core::state::Module {
                    id: "cpu".to_owned(),
                    text: "cpu 30%".to_owned(),
                    accent: Some("starlight".to_owned()),
                    urgent: false,
                },
                realm_core::state::Module {
                    id: "mem".to_owned(),
                    text: "mem 10.0G".to_owned(),
                    accent: None,
                    urgent: false,
                },
                realm_core::state::Module {
                    id: "clock".to_owned(),
                    text: "00:00".to_owned(),
                    accent: None,
                    urgent: false,
                },
            ]
        );
    }

    #[test]
    fn counter_resets_and_interface_changes_never_publish_false_rates() {
        let mem = "MemTotal: 1048576 kB\nMemAvailable: 524288 kB\n";
        let mut sampler = ProcSampler::default();
        let initial = sampler.observe(
            "cpu 10 0 10 80 0 0 0 0\n",
            mem,
            "eth0: 1000 0 0 0 0 0 0 0 1000 0 0 0 0 0 0 0\n",
            Duration::ZERO,
        );
        assert_eq!(initial.snapshot.network, None);

        let valid = sampler.observe(
            "cpu 20 0 20 160 0 0 0 0\n",
            mem,
            "eth0: 3000 0 0 0 0 0 0 0 5000 0 0 0 0 0 0 0\n",
            Duration::from_secs(1),
        );
        assert_eq!(valid.snapshot.cpu_percent, Some(20));
        assert_eq!(valid.snapshot.network.unwrap().receive_bps, 2000);
        assert_eq!(valid.snapshot.network.unwrap().transmit_bps, 4000);

        let reset = sampler.observe(
            "cpu 1 0 1 8 0 0 0 0\n",
            mem,
            "eth0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n wlan0: 7000 0 0 0 0 0 0 0 9000 0 0 0 0 0 0 0\n",
            Duration::from_secs(1),
        );
        assert_eq!(reset.snapshot.cpu_percent, Some(20));
        assert_eq!(reset.snapshot.network, valid.snapshot.network);

        let after_hotplug = sampler.observe(
            "cpu 11 0 11 88 0 0 0 0\n",
            mem,
            "wlan0: 10000 0 0 0 0 0 0 0 14000 0 0 0 0 0 0 0\n",
            Duration::from_secs(2),
        );
        assert_eq!(after_hotplug.snapshot.network.unwrap().receive_bps, 1500);
        assert_eq!(after_hotplug.snapshot.network.unwrap().transmit_bps, 2500);
    }

    #[test]
    fn battery_uevents_select_remove_and_mark_low_capacity_exactly() {
        let root = std::env::temp_dir().join(format!(
            "realm-battery-fixture-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("ZZZ")).unwrap();
        fs::write(root.join("ZZZ/type"), "Battery\n").unwrap();
        fs::write(root.join("ZZZ/capacity"), "80\n").unwrap();
        fs::write(root.join("ZZZ/status"), "Charging\n").unwrap();
        fs::create_dir_all(root.join("AAA")).unwrap();
        fs::write(root.join("AAA/type"), "Battery\n").unwrap();
        fs::write(root.join("AAA/capacity"), "15\n").unwrap();
        fs::write(root.join("AAA/status"), "Discharging\n").unwrap();
        fs::create_dir_all(root.join("AC")).unwrap();
        fs::write(root.join("AC/type"), "Mains\n").unwrap();

        let selected = scan_battery(&root).unwrap().unwrap();
        assert_eq!(selected, BatteryReading::new(15, true));
        assert!(selected.module().urgent);
        fs::remove_dir_all(root.join("AAA")).unwrap();
        assert_eq!(
            scan_battery(&root).unwrap(),
            Some(BatteryReading::new(80, false))
        );
        fs::remove_dir_all(root.join("ZZZ")).unwrap();
        assert_eq!(scan_battery(&root).unwrap(), None);
        fs::remove_dir_all(root).unwrap();

        assert!(super::is_power_supply_uevent(
            b"change@/devices/x\0ACTION=change\0SUBSYSTEM=power_supply\0"
        ));
        assert!(!super::is_power_supply_uevent(
            b"change@/devices/x\0SUBSYSTEM=net\0"
        ));

        let mut sampler = BatterySampler::default();
        assert!(!sampler.observe(Ok(Some(BatteryReading::new(80, false)))));
        assert!(sampler.observe(Err(io::Error::other("transient"))));
        assert!(!sampler.observe(Err(io::Error::other("still transient"))));
        assert_eq!(sampler.reading(), Some(BatteryReading::new(80, false)));
        assert!(!sampler.observe(Ok(None)));
        assert_eq!(sampler.reading(), None);
        assert!(sampler.observe(Err(io::Error::other("new episode"))));
    }

    #[test]
    fn source_failures_retain_last_value_and_log_once_per_episode() {
        let mut sampler = ProcSampler::default();
        sampler.observe(
            "cpu 1 0 1 8 0 0 0 0\n",
            "MemTotal: 1048576 kB\nMemAvailable: 524288 kB\n",
            "eth0: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n",
            Duration::ZERO,
        );
        let good = sampler.observe(
            "cpu 11 0 11 88 0 0 0 0\n",
            "MemTotal: 1048576 kB\nMemAvailable: 524288 kB\n",
            "eth0: 101 0 0 0 0 0 0 0 201 0 0 0 0 0 0 0\n",
            Duration::from_secs(1),
        );
        assert_eq!(good.new_failures, Vec::<Source>::new());
        let first_bad = sampler.observe("broken", "broken", "broken", Duration::from_secs(1));
        assert_eq!(
            first_bad.new_failures,
            vec![Source::Cpu, Source::Memory, Source::Network]
        );
        let repeated = sampler.observe("broken", "broken", "broken", Duration::from_secs(1));
        assert!(repeated.new_failures.is_empty());
        assert_eq!(repeated.snapshot.memory_tenths_gib, Some(5));
        let recovered = sampler.observe(
            "cpu 1000 0 800 2000 0 0 0 0\n",
            "MemTotal: 1048576 kB\nMemAvailable: 524288 kB\n",
            "eth0: 900000 0 0 0 0 0 0 0 700000 0 0 0 0 0 0 0\n",
            Duration::from_secs(1),
        );
        assert!(recovered.new_failures.is_empty());
        assert_eq!(recovered.snapshot.cpu_percent, good.snapshot.cpu_percent);
        assert_eq!(recovered.snapshot.network, good.snapshot.network);
        let bad_again = sampler.observe("broken", "broken", "broken", Duration::from_secs(1));
        assert_eq!(
            bad_again.new_failures,
            vec![Source::Cpu, Source::Memory, Source::Network]
        );
    }

    #[test]
    fn sampler_mailbox_replaces_with_latest_and_try_lock_never_blocks() {
        let mailbox = SamplerMailbox::new().unwrap();
        let first = ModuleSnapshot {
            cpu_percent: Some(10),
            ..ModuleSnapshot::default()
        };
        let second = ModuleSnapshot {
            cpu_percent: Some(80),
            ..ModuleSnapshot::default()
        };
        mailbox.publish(first).unwrap();
        mailbox.publish(second).unwrap();
        assert_eq!(mailbox.try_take_latest().unwrap(), Some(second));

        mailbox.publish(first).unwrap();
        let guard = mailbox.slot.lock().unwrap();
        assert_eq!(mailbox.try_take_latest().unwrap(), None);
        let mut readiness = [PollFd::from_borrowed_fd(
            mailbox.event_fd.as_fd(),
            PollFlags::IN,
        )];
        assert_eq!(
            poll(&mut readiness, Some(&rustix::event::Timespec::default())).unwrap(),
            1
        );
        drop(guard);
        assert_eq!(mailbox.try_take_latest().unwrap(), Some(first));
    }
}
