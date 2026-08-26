# Integration surface — hardware, media and localisation

> **Status: research, not a decision.** This file inventories the OS-level
> services helm has to sit on top of for hardware, media and localisation,
> assesses each candidate against the vision in
> [`design/HANDOFF.md`](../../design/HANDOFF.md) and the budgets in
> [ARCHITECTURE §4](../ARCHITECTURE.md#4-what-robust-and-snappy-mean-here), and
> proposes a priority. Anything here that changes helm's shape needs an ADR;
> this document is the evidence, not the decision.

Companion file: `session-services.md` (notifications, clipboard, portals, idle
and lock, secrets, polkit) is owned separately. Where the two touch — media keys
versus notifications, idle inhibition versus the lock screen — this file says so
and defers.

## How to read the evidence

Claims are labelled. **Verified** means I read the protocol XML, the source, the
API documentation or a package index during this research and can point at it.
**Assumed** means it is well-established practice that I did not re-check, and
it should be treated as a thing to confirm before it is built on. Every
"verified" claim below has an inline link.

Two facts were checked against river's own source tree at
[codeberg.org/river/river](https://codeberg.org/river/river), default branch
`main`, last pushed 2026-08-25, because most of this surface changes shape
depending on whether the compositor or the window manager owns it.

---

## 0. Three cross-cutting findings, before the component list

### 0.1 ADR 0013's protocol table is incomplete: there are six, not four

[ADR 0013](../adr/0013-river-window-management-backend.md) lists four protocols
helm must serve under river: `river-window-management-v1`,
`river-layer-shell-v1`, `river-xkb-bindings-v1` and `river-input-management-v1`.
river's `protocol/` directory contains
[six](https://codeberg.org/river/river/src/branch/main/protocol) (verified):

| Protocol | ADR 0013 lists it | What it actually governs |
|---|---|---|
| `river-window-management-v1` | yes | layout, focus, ordering, borders |
| `river-layer-shell-v1` | yes | the bar existing at all |
| `river-xkb-bindings-v1` | yes | the keymap and the chord model |
| `river-input-management-v1` | yes | seats, keyboard repeat |
| **`river-xkb-config-v1`** | **no** | **the keyboard layout, and switching it** |
| **`river-libinput-config-v1`** | **no** | **touchpad tap-to-click, natural scroll, pointer acceleration, click method, everything libinput exposes** |

Both missing protocols are in *this* document's scope, and both matter for the
week test:

- [`river-xkb-config-v1`](https://codeberg.org/river/river/raw/branch/main/protocol/river-xkb-config-v1.xml)
  (interfaces at version 2, verified) has `create_keymap(fd, format)`,
  `set_keymap`, `set_layout_by_index`, `set_layout_by_name`, a `layout(index,
  name)` **event**, and explicit `capslock_enable`/`disable`,
  `numlock_enable`/`disable` with matching `capslock_enabled` /
  `capslock_disabled` events. Without helm implementing it, the only layout a
  user gets is river's fallback: river builds a default keymap by passing `null`
  to `xkb_keymap_new_from_names`, which means `XKB_DEFAULT_LAYOUT`,
  `XKB_DEFAULT_VARIANT` and `XKB_DEFAULT_OPTIONS` from the environment
  ([`XkbConfig.zig`](https://codeberg.org/river/river/src/branch/main/river/XkbConfig.zig),
  verified). So a fixed non-US layout is reachable through the session
  environment; *switching* layouts, and the bar showing which one is live, is
  not.
- [`river-libinput-config-v1`](https://codeberg.org/river/river/raw/branch/main/protocol/river-libinput-config-v1.xml)
  (901 lines, interfaces at version 2, verified) is the whole libinput surface:
  `set_tap`, `set_tap_button_map`, `set_drag`, `set_drag_lock`,
  `set_three_finger_drag`, `set_natural_scroll`, `set_accel_profile`,
  `set_accel_speed`, `set_click_method`, `set_middle_emulation`,
  `set_scroll_method`, `set_scroll_button`, `set_left_handed`,
  `set_calibration_matrix`, `set_send_events` — each paired with `_support`,
  `_default` and `_current` events, so helm is *told* what the hardware can do
  rather than guessing. Without helm implementing it, every libinput default
  stands. libinput's default for tap-to-click is off on most touchpads
  (assumed — the exact per-device default comes from libinput's quirks
  database), which means **a laptop user cannot tap to click, cannot use
  natural scrolling and cannot change pointer speed**, with no configuration
  file anywhere that would fix it, because river 0.4 has no configuration file:
  the window manager is the configuration.

This is a scope finding for M2, not a discovery about hardware daemons — but it
lands here because touchpads and keyboard layouts are hardware and
localisation. **Recommendation: an ADR amendment adding the two protocols to
0013's obligation table, and M2 issues for each.**

### 0.2 The bar cannot be a pure function of `HelmState` unless hardware state is *in* `HelmState`

[INTERFACES §3](../INTERFACES.md) says the bar is "a pure function of
`HelmState` plus the palette" and owns no state. The mockup's bar shows net
throughput, cpu, mem, gpu temperature, volume and battery — none of which are
window-manager state. There are only two coherent answers:

1. Each bar module opens its own D-Bus/PipeWire/netlink connection. This breaks
   the purity claim, puts sockets in a process whose whole design is "dumb
   renderer", and means a restarted bar re-subscribes to eight services.
2. `helm-session` subscribes once, folds the results into `HelmState`, and
   broadcasts as it already does over the NDJSON socket (ADR 0003, ADR 0004).
   The bar stays pure, `helmctl` can print the same values, and a crashed bar
   costs nothing.

**Recommendation: option 2** — but with a hard constraint that follows from ADR
0013's liveness requirement. `helm-session` is on river's input path and a stall
is an `unresponsive` error, so no D-Bus round trip, no PipeWire round trip and
no `/proc` read may happen on the window-management event loop. The hardware
subscriptions belong on their own thread (or their own `tokio` task set) feeding
a bounded, never-blocking channel; if the channel is full, samples are dropped,
not queued. That constraint is the reason this is worth an ADR rather than an
implementation detail.

### 0.3 TUI tools are themeable for free; GTK tools are not

Everything in this document that has a TUI — `wiremix`, `impala`, `bluetui`,
`nmtui`, `btop`, `yazi` — renders through the terminal's 16-colour ANSI scheme,
which helm already generates from `palette.toml` at M1. A TUI picker in a
floating foot window is *already* helm-coloured with no additional template. The
GTK equivalents (`pavucontrol`, `blueman`, `nm-connection-editor`,
`gnome-control-center`) need the `gtk.css` template, will still show libadwaita
geometry helm cannot change ([`HANDOFF.md`](../../design/HANDOFF.md) says so
explicitly), and drag a toolkit into a session that otherwise has none.

That is a much stronger argument for the TUI options than "they are lighter",
and it applies uniformly below.

---

## 1. Audio — PipeWire, WirePlumber, and what `♪ 64%` talks to

**What it provides.** Sound. Device and stream routing, per-application volume,
Bluetooth audio endpoints, screen-share audio capture, hot-plugged headsets and
docks. PipeWire is the media server; [WirePlumber](https://pipewire.pages.freedesktop.org/wireplumber/)
is the session manager that applies policy (which sink is default, what happens
when a headset appears).

**Without it.** No sound at all, and no volume key that does anything. There is
no "reduced functionality" version of this: a desktop with no volume control
fails the week test on day one, in a meeting.

**Must helm provide, integrate, or not break?** **Integrate.** PipeWire and
WirePlumber are started as systemd user units by the distribution, not by helm.
helm's obligations are three: (a) not to break the user-unit ordering — this is
the sibling file's session-contract territory; (b) to read the default sink's
volume and mute state for the bar; (c) to *change* them, because under river the
window manager owns every key binding, including `XF86AudioRaiseVolume`. That
last point is easy to miss: on a conventional desktop, media keys are handled by
a settings daemon. Under `river-xkb-bindings-v1` there is no settings daemon —
helm binds the key or nothing does.

**Candidates.**

| Option | Language | Status (verified) | Notes |
|---|---|---|---|
| [`pipewire` crate](https://crates.io/crates/pipewire) (pipewire-rs) | Rust | v0.10.1, updated 2026-08-19 | Official bindings. [`Node::subscribe_params` plus a `param` callback](https://pipewire.pages.freedesktop.org/pipewire-rs/pipewire/node/struct.Node.html) is a genuine push interface for volume — verified |
| [`wireplumber.rs`](https://crates.io/crates/wireplumber) | Rust | v0.0.1, updated **2023-08-06** | Effectively dormant. Do not build on it |
| `wpctl` / `pactl` shelled out | — | ships with PipeWire | Fine for a key binding action, wrong for a bar module: a process spawn per keypress is ~2 ms of fork/exec against a 4 ms input-path budget |
| [`wiremix`](https://crates.io/crates/wiremix) | Rust, ratatui | v0.11.0, 2026-06-05 | TUI mixer, an ncpamixer clone. Packaged in Arch, Fedora 41–43 and nixpkgs; **not in Debian or Ubuntu** (verified via repology) |
| `pulsemixer` | Python, curses | packaged everywhere including Ubuntu 24.04 | The portable fallback for per-app routing |
| `pavucontrol` | C++/GTK | 6.x everywhere | Drags GTK in; the canonical answer everyone reaches for, and the wrong one here |

**Fit.** Excellent for the read/write path: PipeWire pushes parameter changes,
so the `♪ 64%` module is event-driven with no timer, which is exactly what the
bar contract demands. `wiremix` is the right per-app mixer for helm — Rust,
keyboard-driven, ANSI-themed for free — with one caveat that is a packaging
problem rather than a fit problem: it is absent from Debian and Ubuntu, so the
`.deb` either builds it or falls back to `pulsemixer`. `pavucontrol` fits badly
and should not be a dependency.

**Integration cost.** A small Rust subscriber in the session daemon (default
sink node, `Props` param, mute and volume) — a day or two including the
"default sink changed" case, which is a WirePlumber metadata change rather than
a node param change and is the part that people get wrong. Plus keymap entries
for `XF86AudioRaiseVolume`/`Lower`/`Mute`/`MicMute`, config-only. Plus a
`wiremix` package dependency and a keybinding to open it in a floating terminal.

**Priority: MVP.** Volume that responds to the volume key is the floor.
**Milestone: M3** (the bar module can land with M2's bar if the session
subscriber is ready; the key bindings and the mixer are M3).

---

## 2. Networking — NetworkManager, iwd, systemd-networkd, and the VPN readout

**What it provides.** Joining a wireless network, including one with a captive
portal or an enterprise 802.1X profile; switching between wired and wireless
without thinking; VPN up/down; and the connectivity state the bar's net module
and urania's `♆ ping 11ms · vpn ◉ warded` line report.

**Without it.** The user cannot join a wifi network without hand-writing a
`wpa_supplicant.conf` or driving `iwctl`. On a laptop that moves between two
buildings, this ends the week test immediately.

**Must helm provide, integrate, or not break?** **Integrate**, and depend on
one. helm must not ship its own connection manager.

**Candidates.**

| Option | What it is | Status (verified via repology) | Fit |
|---|---|---|---|
| [NetworkManager](https://networkmanager.dev/blog/networkmanager-1-58/) | The general-purpose one: wifi, ethernet, WWAN, VPN plugins, per-connection DNS | 1.58.1 (Arch), 1.54.3 (Ubuntu 26.04), 1.54.0 (Fedora 43), 1.46 (Ubuntu 24.04). [1.58 released 2026-07-20](https://networkmanager.dev/blog/networkmanager-1-58/) | The only candidate with a real VPN story. Rich D-Bus API. Installed by default on all three targets |
| [iwd](https://wiki.archlinux.org/title/Iwd) | Intel's wireless daemon; wifi only | 3.12 (Arch), 3.10 (Ubuntu 26.04), 2.14 (Ubuntu 24.04) | Cleaner D-Bus API, much lighter, faster to associate. No wired, no VPN, no WWAN |
| systemd-networkd | Declarative wired/static configuration | ships with systemd | Right for a server, wrong for a laptop that roams |
| NM with the iwd backend | NM's API over iwd's supplicant | supported; the [Debian wiki](https://wiki.debian.org/NetworkManager/iwd) has called it experimental, and NM 1.58's notes mention iwd-backend fixes as recently as 2026 | Interesting, but two moving parts for one benefit |

**Front ends.** [`nmtui`](https://networkmanager.dev/) ships with NetworkManager
itself (assumed: as the `NetworkManager-tui` subpackage on Fedora and inside
`network-manager` on Debian and Ubuntu — confirm in packaging CI). It is
keyboard-driven, ugly, and works. [`impala`](https://crates.io/crates/impala) is
a Rust/ratatui TUI over iwd (v0.7.3 on crates.io, 2026-02-03; packaged only in
Arch and nixpkgs — verified), and there is an `impala-nm` crate suggesting a
NetworkManager variant exists. `iwmenu` and `networkmanager-dmenu` drive a
launcher instead of a TUI.

**Fit.** NetworkManager is not a natural fit for helm's aesthetic and it is
still the right answer, because VPN is in the design. urania's `vpn ◉ warded`
readout is a NetworkManager `ActiveConnection` of type `vpn` — reproducing that
over iwd would mean helm implementing WireGuard or OpenVPN management itself.
The D-Bus surface is properly event-driven: `PropertiesChanged` on devices and
active connections, plus `StateChanged`, so the bar's up/down/SSID state costs
no timer (assumed for the exact signal set; the D-Bus interface is
long-standing and stable).

The `↑ 18k ↓ 1.2M` throughput readout is a different matter and is dealt with in
the event-driven audit below: byte counters are not pushed by anything.

**Integration cost.** Config-only for the dependency and the `nmtui` binding.
A small Rust subscriber (zbus) for the bar's connectivity state, plus a second
one for VPN state. If helm wants the picker to look like helm rather than like
nmtui, that is hecate work — see §17.

**Priority: MVP.** The stated example. **Milestone: M3.**

---

## 3. Bluetooth — BlueZ and a keyboard-driven picker

**What it provides.** Pairing and connecting headsets, mice, keyboards and
phones. The audio side is PipeWire's, but the pairing side is BlueZ's.

**Without it.** Bluetooth headphones and mice cannot be paired without
`bluetoothctl`, an interactive prompt-driven tool that is genuinely unpleasant
and which people get wrong (`scan on`, `pair`, `trust`, `connect`, in that
order, with a device address you have to read off a scrolling log).

**Must helm provide, integrate, or not break?** **Integrate.** BlueZ is the only
Bluetooth stack on Linux; `bluetoothd` is a system service the distribution
starts.

**Candidates.**

| Option | Language | Status (verified) | Notes |
|---|---|---|---|
| [`bluetui`](https://crates.io/crates/bluetui) | Rust, ratatui | v0.8.0 crates.io 2025-11-13; 0.8.1 in Arch and nixpkgs; **absent from Debian, Ubuntu and Fedora** | Talks to BlueZ over D-Bus directly. Closest fit |
| [`bluetuith`](https://github.com/darkhz/bluetuith) | Go | 0.2.7 in nixpkgs only | More featureful (OBEX file transfer, PBAP); a Go runtime in the dependency set |
| `bluetoothctl` | C | ships with BlueZ everywhere | The universal fallback; hostile to newcomers |
| `blueman` | Python/GTK | packaged everywhere | Tray-icon oriented, GTK, mouse-driven. Bad fit |

**Fit.** `bluetui` is the right shape and the wrong packaging story: Arch and
Nix only, which means the `.deb` and `.rpm` must build it from crates.io or fall
back to `bluetoothctl`. BlueZ's D-Bus API is `ObjectManager`-based
(`InterfacesAdded`, `InterfacesRemoved`, `PropertiesChanged`), so a
connected/disconnected indicator is event-driven if helm ever wants one — the
mockup does not have a Bluetooth module in the bar, and it should stay that way
unless a device is connected.

**Integration cost.** Config-only plus a package dependency, plus one keymap
entry. If `bluetui` cannot be packaged for Debian, either vendor the build or
accept `bluetoothctl` there and say so in the install docs.

**Priority: MVP**, on cost grounds rather than importance grounds — a keybinding
and a dependency is close to free, and "my headphones will not pair" is a
plausible week-ending failure. **Milestone: M3.**

---

## 4. Display and output configuration

**What it provides.** Turning on the external monitor when it is plugged in;
placing it left or right; per-output scale on a HiDPI laptop next to a 1080p
projector; refresh rate; and remembering all of that per docking situation.

**Without it.** Plug in a projector and either nothing happens or it mirrors at
the wrong resolution, with no command to fix it.

**Must helm provide, integrate, or not break?** Split, and the split is the
interesting part.

- **helm must handle hotplug**, because the workarea changes.
  `river_output_v1` delivers `position` and `dimensions` events to the window
  manager, and an `output`/`removed` pair on the manager
  ([`river-window-management-v1.xml`](https://codeberg.org/river/river/raw/branch/main/protocol/river-window-management-v1.xml),
  verified). ADR 0013 already maps this to `Workarea`. Hotplug arrives as a
  protocol event; there is nothing to poll and no excuse for getting it wrong.
- **helm must not configure outputs**, because it cannot. I read the
  `river_output_v1` interface in full: its requests are `destroy` and
  `set_presentation_mode`; its events are `removed`, `wl_output`, `position`,
  `dimensions` and `capture_sessions` (verified). **There is no scale, no mode,
  no transform and no enable/disable.** Output configuration under river is
  `wlr-output-management-unstable-v1`, which river implements in the compositor
  via wlroots (`OutputManager.zig` creates `wlr.OutputManagerV1` and handles
  `manager_test`/`manager_apply` — verified), and which is spoken by ordinary
  clients.

So the tool that positions monitors is a normal client, not part of helm, and
this is a clean seam rather than a gap.

**Candidates.**

| Option | Language | Status (verified) | Notes |
|---|---|---|---|
| [`kanshi`](https://repology.org/project/kanshi/versions) | C | 1.9.0 newest; Ubuntu 24.04 has 1.5.1, Fedora 43 has 1.8.0, Ubuntu 26.04 has 1.9.0 | Profile-based: match a set of connected outputs, apply a layout. Packaged on every target |
| [`shikane`](https://crates.io/crates/shikane) | Rust | v1.1.1, 2026-06-10; **Arch and nixpkgs only** | kanshi's model with regex matching and better ordering. Right language, wrong packaging |
| `wlr-randr` | C | packaged everywhere | One-shot imperative changes; the right thing for a keybinding or `helmctl` to shell out to |
| `wdisplays` / `nwg-displays` | C/GTK, Python/GTK | packaged | Graphical, mouse-driven. Against the grain of the whole project |

**Fit.** kanshi's config file is declarative, static and lives beside helm's
other generated configs — a good fit for a DE that already believes in
generated config. shikane is the better Rust citizen and costs two extra
packaging jobs; that trade is a judgement call, and kanshi's presence on all
three distros makes it the MVP answer.

**Fractional scaling is a live risk to helm's central invariant.** river rounds
each output's scale to the nearest 1/120 so it is exactly representable in
`fractional-scale-v1`, and hands the window manager logical dimensions computed
as physical divided by scale (`Output.zig`, verified). helm's exact-tiling
guarantee is over *logical* integers. At scale 1.25 or 1.5, an exact logical
partition can still land on a half-physical-pixel boundary, and the compositor —
not helm — decides how that rounds when it draws the 1px border. `PITFALLS.md`
has a row for fractional scaling blur with a planned M2 test; that test should
assert seam exactness at 1.25, 1.5 and 1.75 as well as at 1.0, or the pitfall
is only half-closed.

**Integration cost.** Config-only for kanshi (plus a generated profile stanza if
helm ever wants to own it). Hotplug handling inside `RiverBackend` is real work
but is already M2's, not new.

**Priority: MVP** for "hotplug does not break and the workarea updates";
**LATER** for multi-monitor comfort, which `MVP.md` already defers.
**Milestone: M3** for the kanshi dependency, **M6** for real multi-monitor
support.

---

## 5. Brightness and backlight, keyboard backlight

**What it provides.** Screen brightness on the brightness keys; keyboard
backlight on its key.

**Without it.** A laptop is unreadable in sunlight and blinding at night, and
the two dedicated keys on the keyboard do nothing. This is one of the most
immediately noticeable absences in any minimal desktop.

**Must helm provide, integrate, or not break?** **Provide the binding, integrate
with logind.** As with volume, there is no settings daemon under river: helm
binds `XF86MonBrightnessUp` or nobody does.

**Candidates.**

| Option | Mechanism | Notes |
|---|---|---|
| **systemd-logind `SetBrightness`** | D-Bus, on `org.freedesktop.login1.Session`, signature `SetBrightness(in s subsystem, in s name, in u brightness)`, subsystem `"backlight"` or `"leds"` | **No polkit privilege required**; the caller must own the session and the session must be active ([verified against the org.freedesktop.login1 manual](https://manpages.debian.org/testing/systemd/org.freedesktop.login1.5.en.html)). Available since systemd 243, so every target qualifies (Ubuntu 24.04 ships 255). **This is the right answer**: pure zbus, no extra binary, no setuid, no udev rule, and it covers the keyboard backlight through the same call with `"leds"` |
| [`brightnessctl`](https://repology.org/project/brightnessctl/versions) | Small C tool; uses logind when built with it, else a setuid helper or udev rule | 0.5.1 on every target including Ubuntu 24.04 (verified). The pragmatic fallback and the thing `helmctl doctor` can point at |
| [`brightness` crate](https://crates.io/crates/brightness) | Rust, sysfs | v0.8.0, 2025-09-02. Cross-platform; wants write access to sysfs, which is the problem logind exists to solve |
| `ddcutil` | I²C to external monitors over DDC/CI | 2.2.7, packaged everywhere. The *only* way to set an external monitor's brightness. Slow (tens of milliseconds per transaction), needs `i2c-dev` and group membership |
| [`wluma`](https://repology.org/project/wluma/versions) | Rust, adaptive brightness from ambient light and screen contents | 4.11.1, nixpkgs only. Uses screen capture, so it costs power continuously. Interesting, wrong for a zero-idle-CPU project |

**Fit.** logind's `SetBrightness` is close to a perfect fit: one D-Bus call from
a crate helm already needs, no privilege escalation, no polling. Reading the
current value still means reading `/sys/class/backlight/*/brightness`, and
changes made by the firmware (the hardware brightness keys on some laptops act
below the OS) surface as udev `change` events on the `backlight` subsystem
(assumed — worth confirming with `udevadm monitor` on real hardware before the
bar shows a brightness value at all; the mockup does not have a brightness
module, which conveniently sidesteps this).

**Integration cost.** A small Rust shim: two keymap actions and a zbus call.
Half a day. External-monitor brightness via ddcutil is a separate, optional,
clearly-signposted feature — not MVP.

**Priority: MVP.** **Milestone: M3.**

---

## 6. Power and battery — upower, power profiles, suspend, lid

**What it provides.** The `⚡ 87%` readout and urania's `87% cell`; a warning
before the machine dies; suspend on lid close; a performance/balanced/power-save
switch.

**Without it.** No battery indicator, no low-battery warning (the laptop simply
switches off mid-sentence), and a lid that closes without suspending — which is
a security problem as much as a battery one.

**Must helm provide, integrate, or not break?** **Integrate**, with one thing to
*not break*: lid handling belongs to logind (`HandleLidSwitch` in
`logind.conf`), and it works whether or not helm does anything, provided helm
does not take a permanent sleep inhibitor. The classic bug is a desktop that
inhibits sleep "while the session is active" and then never releases it.

**Candidates.**

| Concern | Component | Interface | Status |
|---|---|---|---|
| Battery state | [UPower](https://upower.freedesktop.org/) | `org.freedesktop.UPower`, the `DisplayDevice` aggregate at `/org/freedesktop/UPower/devices/DisplayDevice`, with `Percentage`, `TimeToEmpty`, `State`, `WarningLevel` (Unknown/None/Discharging/Low/Critical/Action) and `PropertiesChanged` | 1.90–1.91 on every target (verified). Fully push-based |
| Low-battery warning | UPower `WarningLevel` | as above | Using `WarningLevel` rather than a hardcoded percentage is the documented way; the thresholds are UPower's configuration, not helm's invention |
| Power profiles | [power-profiles-daemon](https://repology.org/project/power-profiles-daemon/versions) 0.21–0.30, or [tuned-ppd](https://fedoraproject.org/wiki/Changes/TunedAsTheDefaultPowerProfileManagementDaemon) | same D-Bus API — tuned-ppd is explicitly a drop-in translation layer, which is why Fedora could switch defaults from ppd to tuned in F41 without desktops changing code | Integrate against the API, not the implementation, and both distros are covered |
| Suspend/hibernate/lid | systemd-logind | `Manager.Suspend()`, `PrepareForSleep` signal, `Inhibit()` with `block`/`delay` modes; `HandleLidSwitch*` properties (verified from the manual) | Nothing to build. `PrepareForSleep(true)` is where a lock screen hooks in — sibling file's territory |
| Battery notifications | [`poweralertd`](https://repology.org/project/poweralertd/versions) 0.3.0 (Debian 13, Ubuntu 25.10+, nixpkgs; **not in Fedora**) | UPower to notifications | The off-the-shelf option, and it depends on a notification daemon, which is the sibling file's decision |

**Fit.** Excellent. This is the most event-driven corner of the whole surface:
UPower pushes, logind pushes, power-profiles-daemon pushes. A battery module
with a timer in it would be a bug, not a compromise.

One helm-specific wrinkle: a low-battery *warning* needs somewhere to appear.
The mockup has no notification surface — urania has a "wards" footer and the bar
has a battery module that could turn gold and then red via `palette.toml`'s
`accent.gold`. Recolouring the existing module is free, requires no notification
daemon, and matches "no wasted space". At `WarningLevel = Action`, something
more assertive is warranted; that is a design decision worth taking
deliberately rather than by default.

**Integration cost.** A small Rust subscriber (zbus, UPower) plus palette
thresholds. Power profiles are a `helmctl power` verb and one more D-Bus proxy —
cheap, and not MVP.

**Priority: MVP** for battery percentage, warning level and lid-close suspend.
**STRATEGIC** for power profiles. **Milestone: M3** / **M4**.

---

## 7. Input — layouts, input methods, pointers

**What it provides.** Typing in your own language; switching between two
layouts; entering Chinese, Japanese or Korean; a touchpad that behaves the way
you configured it a decade ago and never think about.

**Without it.** Covered in §0.1: no layout switching, no bar indicator, and no
touchpad configuration at all.

**Must helm provide, integrate, or not break?**

- **Layouts and pointer configuration: helm must provide**, because under river
  0.4 the window manager *is* the input configuration (§0.1).
- **Input methods: not break**, and that is genuinely all — but it needs
  checking rather than assuming, and the check came out well.

**Input methods, verified.** river's `InputManager` creates
`wlr.InputMethodManagerV2` and `wlr.TextInputManagerV3`, and there is a full
`InputRelay.zig` plus `InputPopup.zig` handling `InputMethodV2.KeyboardGrab` and
`InputPopupSurfaceV2` ([verified in source](https://codeberg.org/river/river/src/branch/main/river/InputRelay.zig)).
So `input-method-v2` **and** the candidate-window popup work on river. That
matters: sway supports input-method-v2 without popups, which is why sway users
[cannot use Chinese input methods that need a candidate window](https://wiki.archlinux.org/title/Fcitx5).
river does not have that limitation. fcitx5 is at 5.1.19 on Ubuntu 26.04, 5.1.21
on Fedora 43 (verified), and fcitx5's own issue tracker records river being
fixed upstream for the 5.1.9 `zwp_input_method_v2` regression ahead of sway.

The remaining caveat is terminal-shaped and lands squarely on helm, because helm
is a terminal-centric desktop: [foot supports only pure-Wayland IMEs and
requires the compositor to implement `text-input-v3`](https://codeberg.org/dnkl/foot/issues/345)
— which river does — so CJK input in foot should work. That "should" is the
honest word: I did not test it, and it is exactly the kind of claim that needs a
`doctor` check or a manual verification note before helm claims CJK support.

**Candidates.**

| Concern | Option | Notes |
|---|---|---|
| Layout, switching, caps/num lock | `river-xkb-config-v1`, implemented by helm | Gives a `layout(index, name)` **event** — the bar's layout indicator is push-driven, free of timers |
| Fixed layout without protocol work | `XKB_DEFAULT_LAYOUT` etc. in the session environment | Verified fallback path. Must be added to ADR 0011's import list, or a non-US user's layout will not survive into anything the bus activates |
| Touchpad, mouse | `river-libinput-config-v1`, implemented by helm | The `_support` events mean helm can grey out or refuse settings the hardware cannot do, rather than silently ignoring them |
| CJK | fcitx5 (5.1.x, all targets) or ibus | fcitx5 is the better Wayland citizen in 2026. Integration is environment variables plus an autostarted user unit |
| On-screen keyboard | `squeekboard` (1.43.1, Arch/Ubuntu 26.04/nixpkgs) | Only relevant on touch hardware, which helm does not target. Note for accessibility, §12 |

**Fit.** Implementing two more protocols is unwelcome scope, and it is also the
price of ADR 0013 — the same bargain that gave helm frame-perfect layout gave it
the input configuration to own. The protocol shapes are helm-friendly: they are
declarative setters with capability events, which maps onto a `[input]` section
in helm's own config and a `helmctl input` verb. There is nothing to poll.

The keymap indicator in the bar is a design gap worth naming: `HANDOFF.md`'s bar
has no layout module, because the design assumes one layout. Anyone with two
layouts needs to see which is active; the mode badge area is the natural place,
and it costs one more `HelmState` field.

**Integration cost.** Real work — two protocol implementations in
`helm-session`, plus a config surface for them, plus the fcitx5 environment
wiring (config-only). This is the largest single item in this document.

**Priority: MVP** for layout selection and touchpad tap/scroll/accel — a
laptop without tap-to-click and a Colemak user with no way to say so both fail
the week test. **STRATEGIC** for runtime layout switching with a bar indicator.
**LATER** for CJK verification as a supported, tested configuration.
**Milestone: M2** for the protocols themselves (they belong with the other four),
**M3** for the config surface, **M4** for the indicator.

---

## 8. Removable media — udisks2 and charon

**What it provides.** Plug in a USB stick, and it is mounted somewhere you can
reach; unplug it safely; unlock a LUKS volume.

**Without it.** `udisksctl mount -b /dev/sdb1` — or worse, `sudo mount`. A file
manager that cannot open a USB stick is a file manager with a hole in it.

**Must helm provide, integrate, or not break?** **Integrate.** udisks2 is a
system D-Bus service (2.10–2.11 on all targets, verified) that already handles
the privilege problem via polkit; the desktop's job is to ask it to mount things
and to notice when a device appears.

**Candidates.**

| Option | Language | Status | Notes |
|---|---|---|---|
| [`udiskie`](https://github.com/coldfix/udiskie) | Python | 2.7.0 released 2026-07-29 (verified via PyPI); 2.5.x–2.6.x in the distros | The standard automounter. Runs headless with `--no-tray`; the tray icon and notifications are optional. Python in the dependency set |
| [`udisks2` crate](https://crates.io/crates/udisks2) | Rust, zbus | v0.3.1, 2026-01-21 | Bindings, not a daemon. The building block if helm writes its own ~200-line automounter |
| `udevil` / `pmount` | C | mostly unmaintained | No |
| yazi plugins | Lua | community plugins exist for mount/unmount | The natural place for the *manual* path, and it keeps the interaction inside charon where the user already is |

**Fit.** udisks2's `ObjectManager` interface (`InterfacesAdded`) is push-based,
so an automounter is an event loop, not a poll loop. udiskie fits helm's *needs*
badly only in that it is Python and tray-shaped; run with `--no-tray` under a
systemd user unit it is invisible and works. Writing a Rust replacement is
tempting and is roughly a weekend, but it is a weekend spent on something
udiskie already does correctly including LUKS and multi-partition devices —
ADR 0007's doctrine says reuse it and say why if we ever stop.

The charon surface is the interesting half: yazi should show mounted removable
volumes as jump targets, and mounting should be a keybinding rather than a
shell-out the user has to remember. That is `configs/yazi/` work, which is
exactly the seam ADR 0007 describes.

**Integration cost.** Config-only (a session unit for udiskie plus a yazi
keymap entry and a bookmark generator).

**Priority: MVP**, on cost grounds: the fix is a systemd user unit, and "I
plugged in my backup drive and nothing happened" is a quiet, memorable failure.
**Milestone: M3.**

---

## 9. Printing — CUPS, and whether a DE owes anything in 2026

**What it provides.** Printing.

**Without it.** Nothing, in practice, for a desktop environment. Applications
print through their own dialogs — GTK's, Qt's, the browser's — and those talk to
CUPS directly, discovering IPP printers over mDNS without any per-desktop
component. There has been no need for a desktop-supplied print dialog since
driverless printing became the norm.

**Must helm provide, integrate, or not break?** **Not break.** Concretely: do
not break Avahi/mDNS resolution, and make sure the GTK and Qt print dialogs open
— which is the same portal and toolkit-theming question the sibling file covers
for file dialogs.

**State of the art, verified.** [libcups v3.0.0 shipped on 2026-01-08](https://openprinting.github.io/libcups-3.0.0/)
and [CUPS 3.0's design](https://github.com/OpenPrinting/cups/wiki/CUPS-3.0)
drops PPDs entirely in favour of IPP Everywhere plus Printer Applications, with
a per-user local server. That is the future; it is not the present on helm's
targets, which all still ship CUPS 2.4.x (2.4.7 on Ubuntu 24.04, 2.4.11 on
Fedora 42, 2.4.19 on Arch — verified). So helm should assume 2.4 behaviour and
should not build anything that would need reworking for 3.0.

**Fit.** The best fit is nothing at all. A printer-management TUI would be a
project; `system-config-printer` is Python/GTK and increasingly vestigial; the
CUPS web interface on `localhost:631` is the honest fallback and should simply
be documented.

**Integration cost.** Documentation, and a `doctor` line that says whether
`cups.service` is running if the user asks.

**Priority: LATER.** The brief's own example, and correct. **Milestone: M6**, or
never — a paragraph in the install docs would discharge the obligation.

---

## 10. Colour — night light, ICC, HDR

**What it provides.** Warmer colours after sunset; correct colours on a
calibrated display; HDR.

**Without it.** Bright blue light at 23:00 — a comfort feature that people
notice the absence of immediately, and which urania's mockup explicitly promises
(`20:14 ☉ sunset · night palette engages`).

**Must helm provide, integrate, or not break?** **Integrate** for night light.
**Not break** for ICC and HDR, which are out of reach under river anyway.

**Night-light candidates.**

| Option | Language | Status (verified) | Notes |
|---|---|---|---|
| [`wl-gammarelay-rs`](https://github.com/MaxVerevkin/wl-gammarelay-rs) | Rust | 1.0.1 in nixpkgs; crates.io release stale at 0.3.2 (2023) while the repo has moved on — the packaging, not the project, is what to trust here | Exposes a **D-Bus interface** (`rs.wl.gammarelay`, methods `UpdateTemperature`, `UpdateBrightness`, `UpdateGamma`, `ToggleInverted`). That makes colour temperature a helm keybinding and a helm state value rather than a daemon with opinions. Single-threaded, no runtime dependencies. **Best fit** |
| [`wlsunset`](https://repology.org/project/wlsunset/versions) | C | 0.4.0; Ubuntu still on 0.3.0 | Sunrise/sunset by latitude and longitude, no D-Bus. Simple, on every target |
| [`gammastep`](https://repology.org/project/gammastep/versions) | C | 2.0.11; 2.0.9 on Ubuntu/Debian | redshift's Wayland fork; geoclue support if you want automatic location |

All three need `wlr-gamma-control-unstable-v1`, which river creates in
`OutputManager.zig` (`wlr.GammaControlManagerV1`, then
`scene.setGammaControlManagerV1`) — **verified**, so all three work.

**ICC and HDR, verified.** river creates `wlr.ColorManagerV1` at version 2, with
parametric and mastering-display-primaries features — but only inside
`if (renderer.features.input_color_transform)`, i.e. only on a renderer that
supports input colour transforms, which in wlroots means the Vulkan renderer
([`Server.zig`](https://codeberg.org/river/river/src/branch/main/river/Server.zig)).
`color-management-v1` is a staging protocol in
[wayland-protocols](https://gitlab.freedesktop.org/wayland/wayland-protocols)
(verified — it is in `staging/color-management`), and
[wlroots merged support](https://www.phoronix.com/news/wlroots-color-management).
However: I grepped river's `Output.zig` for HDR, ICC, bit-depth, primaries and
transfer-function handling and **found none** (verified). sway exposes HDR
through its own config (`render_bit_depth 10`, `hdr`, `color_profile icc`);
river 0.4 has no config file, and `river_output_v1` has no colour requests, so
there is currently **no way for a helm user to enable HDR on river**.

**Fit.** wl-gammarelay-rs fits helm unusually well: a D-Bus knob means the night
palette can be a helm state transition (`helmctl night on`) that also swaps the
`palette.toml` contrast variant, making the mockup's "night palette engages"
literally true rather than decorative. It is not packaged outside nixpkgs, so
Debian and Fedora would build it from source or fall back to wlsunset.

**Integration cost.** Config-only for wlsunset. A small Rust shim plus a
`helmctl` verb for the wl-gammarelay-rs version, plus the sunset calculation —
the [`sunrise` crate](https://crates.io/crates/sunrise) (v3.0.0, 2026-01-01)
does the astronomy, and urania needs it anyway for `☉ sets 20:14`.

**Priority: STRATEGIC** for night light — the design promises it, the week test
survives without it. **LATER / not applicable** for ICC and HDR: helm cannot
deliver them on river, and should say so rather than implying otherwise.
**Milestone: M4** (night light), **M5** for HDR if `helm-compositor` chooses to
support it.

---

## 11. Locale, timezone and time synchronisation

**What it provides.** The clock being right, including across a DST boundary and
a flight; dates and sort orders in the user's language; `LANG` reaching every
process.

**Without it.** A clock that is an hour wrong twice a year, and applications
that disagree with each other about the date format because some inherited
`LANG` and some did not.

**Must helm provide, integrate, or not break?** **Integrate**, plus one concrete
gap to close.

**The gap.** [ADR 0011](../adr/0011-session-integration-contract.md)'s import
lists name `WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE
XDG_SESSION_DESKTOP XDG_RUNTIME_DIR DISPLAY XCURSOR_THEME XCURSOR_SIZE`. They do
not name `LANG`, `LC_*` or the `XKB_DEFAULT_*` variables from §7. If the user's
locale comes from `~/.config/locale.conf` or a shell profile rather than from
PAM, D-Bus-activated services get the C locale while the terminal gets the right
one, and the symptom — one app sorting `ä` differently from another, or a date
in the wrong format in a file dialog — is nearly impossible to attribute. This
is the same class of bug ADR 0011 exists to prevent, and the fix is adding names
to two lists.

**Candidates.**

| Concern | Component | Interface | Notes |
|---|---|---|---|
| Timezone | `systemd-timedated` | `org.freedesktop.timedate1`, property `Timezone`; **changes to `Timezone` and `LocalRTC` do emit change signals**, while `NTPSynchronized` and `TimeUSec` are annotated `EmitsChangedSignal("false")` ([verified](https://manpages.debian.org/testing/systemd/org.freedesktop.timedate1.5.en.html)) | So "the timezone changed, reload the clock" is push-driven. `SetTimezone` needs polkit `org.freedesktop.timedate1.set-timezone` |
| Time sync | `systemd-timesyncd` or `chrony` | `NTP`/`NTPSynchronized` properties | Distribution's business. A `doctor` line at most |
| Clock arithmetic | [`jiff`](https://crates.io/crates/jiff) v0.2.35 (2026-07-25) or `chrono` + `chrono-tz` | — | jiff handles the tzdb and DST transitions properly and can be told to re-read the zone; it is the better choice for a clock that must survive `SetTimezone` without a restart |
| Sidereal time, moon phase, sunset | urania's readouts | computed | Pure maths plus the `sunrise` crate. Scheduled at the next boundary rather than polled |

**Fit.** Good, and cheap. The bar's clock format is fixed by the design
(`26·08·2026 ☾ 14:32`), which sidesteps locale-dependent formatting entirely —
a deliberate simplification worth keeping.

**Integration cost.** Config-only for the environment fix; a small Rust shim for
the timedated subscription.

**Priority: MVP** for a clock that honours the system timezone and DST, and for
the `LANG`/`XKB_DEFAULT_*` import fix. **LATER** for any UI that *changes* the
timezone. **Milestone: M3.**

---

## 12. Accessibility

This section is longer than the others because the honest answer is worse than
the others, and because a keyboard-first desktop makes a specific promise to
some disabled users and a specific problem for others.

**What it provides.** A screen reader for blind users; magnification for
low-vision users; sticky keys, slow keys and bounce keys for users who cannot
hold two keys at once or who trigger repeats involuntarily; and, less
formally, the ability to use the desktop entirely without a keyboard — which is
the case for many users with severe RSI or motor impairments, who drive their
machines by voice or by switch access.

**Without it.** A blind user cannot use helm at all. Not "with difficulty" — at
all.

**What the research actually says, and what I could and could not verify.**

- **Screen reading.** Orca works on GNOME and on Plasma 6. On wlroots-based
  compositors it does not, and the reason is structural: there is no AT-SPI
  integration in the compositor, and Orca depends on being able to grab keys and
  route events in a way wlroots compositors do not offer
  ([summary](https://github.com/splondike/wayland-accessibility-notes),
  [background](https://blogs.gnome.org/a11y/2024/06/18/update-on-newton-the-wayland-native-accessibility-project/)).
  I verified the structural half directly: **there is no accessibility protocol
  in `wayland-protocols`** — I listed the `staging/` directory and it contains
  `ext-idle-notify`, `ext-session-lock`, `color-management` and twenty-seven
  others, and nothing for accessibility. Newton, the Wayland-native
  accessibility architecture built on AccessKit, remains the plan of record and
  is not a shipping cross-desktop protocol as of August 2026. I could **not**
  verify a specific "Orca on river" report either way; my claim is that it does
  not work, based on the absence of the mechanism rather than on a bug report.
- **Magnification.** GNOME's magnifier is compositor-level. Under river, helm
  cannot implement magnification at all: helm positions windows and river
  renders them, and `river-window-management-v1` has no zoom, no transform and
  no scale request (verified — the `river_output_v1` interface listing in §4).
  So screen magnification is **architecturally impossible before M5**, when helm
  owns `helm-compositor`. That is a real finding and it should be written down
  rather than discovered by a user.
- **Sticky keys, slow keys, bounce keys.** These are AccessX features. On X11
  the server implemented them; on Wayland the compositor must, and libxkbcommon
  does not do it for you. GNOME and KDE implement them; wlroots compositors do
  not (assumed for river specifically — I found no AccessX handling in river's
  input files, but I did not read `Keyboard.zig` in full). Worse for helm: under
  `river-xkb-bindings-v1`, helm sees *bound* keys, not the raw key stream, so
  helm probably could not implement sticky keys itself even if it wanted to.
  That needs confirming against the protocol before anyone promises otherwise.
- **The keyboard-first promise cuts both ways.** For a user with low vision who
  can type, helm is unusually good: everything is reachable by key, there are no
  animations (which matters for vestibular disorders and for motion-triggered
  migraine), contrast is a first-class derived setting rather than a filter, and
  the layout never moves under you. Those are genuine accessibility wins that
  the project got for free by having good taste, and it is fair to claim them.
  For a user who cannot use a keyboard the way the design assumes — one-handed,
  switch-access, voice-driven, or unable to chord — helm's core interaction
  model is a chorded modifier under `mod`, and that is precisely the interaction
  sticky keys exist to fix. helm cannot currently offer sticky keys. That is not
  a gap in polish; it is the main interaction model being unavailable to a
  group of users.

**Must helm provide, integrate, or not break?** Today, realistically: **not
break, and not lie.** Concretely —

1. **Say so.** The install documentation and `helmctl doctor` should state
   plainly that helm does not support screen readers, and that this is a
   property of the wlroots/river stack, not a decision. A user who needs Orca
   should learn that from helm's own documentation in thirty seconds, not from a
   failed installation.
2. **Do not obstruct what does work.** XWayland-based magnifiers and any
   AT-SPI-based tooling that a user brings should not be actively broken; the
   AT-SPI bus should be allowed to start.
3. **Bank the wins that are free.** Contrast variants across their full range
   (already planned for M6), no motion (already ADR 0009), full keyboard
   reachability (already the design), and a text-size setting that actually
   propagates — `palette.toml` has `size_body` and friends, and fontconfig
   plus the bar honour them, which is more than most tiling desktops offer.
4. **Own the parts that become possible at M5.** Magnification and AccessX
   belong in `helm-compositor`'s requirements from the day its spec is written,
   not as M6 polish. Writing them down now costs nothing and changes what the
   compositor's architecture has to allow.

**Fit.** There is no candidate to assess, which is itself the finding: for the
single most important accessibility feature there is no implementation that
works on this stack at any price.

**Integration cost.** Documentation now; real work at M5.

**Priority: MVP** for the honest statement in the docs and the doctor.
**STRATEGIC** for the free wins. **LATER (M5)** for magnification and AccessX,
as compositor requirements. **Milestone: M3 / M5.**

---

## 13. Fonts — fontconfig and the glyph contract

**What it provides.** Which font a program actually gets when it asks for "IBM
Plex Mono", and what it falls back to for a rune, a planetary symbol or an
Egyptian hieroglyph.

**Without it.** Tofu. [ADR 0012](../adr/0012-font-fallback-is-a-contract.md)
already treats this as a contract and `helm-core::glyphs` already implements the
inventory and probe, so most of the work is done.

**The gap I found.** `palette.toml` declares an ordered `typography.fallback`
chain, and `glyphs::probe()` verifies coverage *in helm's own renderer*
(`cosmic-text`, per ADR 0008). Nothing makes **fontconfig** honour that order,
and fontconfig is what resolves fonts for GTK, Qt, foot, Firefox and every other
program in the session. So helm can pass its own glyph probe while yazi shows a
box, because yazi is a foot client and foot asks fontconfig, which will happily
substitute a colour emoji font for `☾` — the exact failure `PITFALLS.md` lists
as "emoji font hijacks symbols".

**Recommendation: a `fonts.conf` template in `configs/templates/`**, generated
from the same `typography.fallback` list, emitting an ordered `<prefer>` block
for the monospace family and a `<rejectfont>` for colour emoji in the monospace
chain. That makes the fallback chain one source of truth instead of two, which
is ADR 0005's whole thesis applied to fonts.

**Candidates.** fontconfig is the only implementation (2.15 on Ubuntu 24.04,
2.17.1 on Ubuntu 26.04, 2.17.0 on Fedora 43, 2.18.x on Arch and nixpkgs —
verified). There is no alternative and no need for one.

**Fit.** Perfect: a generated config file is exactly the mechanism helm already
has.

**Integration cost.** One template plus one test that renders it and parses it
with `fc-conflist` — the same shape as ADR 0007's planned per-tool render tests.

**Priority: MVP.** The glyph contract is not closed without it, and the failure
is visible in the first frame. **Milestone: M3** (M1 if it lands with the other
templates).

---

## 14. Thumbnails — the cost of "never"

The design is unambiguous: charon's header reads `thumbnails: never`, and
`HANDOFF.md` says "No thumbnails, ever; previews are text/stat only and
toggleable". `ARCHITECTURE.md`'s cold-start budget names "no thumbnailer" as one
of the reasons the session starts in under 900 ms.

**What it costs, concretely.** yazi does not need a thumbnailer bolted on: it
has built-in image preview over the [Kitty graphics protocol, Sixel, iTerm2 and
Ghostty protocols](https://yazi-rs.github.io/docs/image-preview/), and **foot
supports Sixel**, which means image preview works out of the box in helm's
default terminal (verified from yazi's documentation; the foot-specific path is
documented by yazi as Sixel). So "never" is not helm avoiding a feature that
would be expensive to build — it is helm switching off a feature that is
already there and already fast.

The people who lose are specific: anyone triaging a directory of screenshots,
anyone choosing between twelve photos, anyone with a folder of PDFs whose
filenames are hashes, anyone doing design work. For them, "no thumbnails" means
opening files one at a time in an image viewer (`swayimg` is packaged
everywhere, 4.7 on Fedora 43 — verified) to do what a grid of thumbnails does at
a glance. That is not a small tax; it is the main way people navigate media.

**Is it defensible?** As a *default*, yes, and for good reasons: thumbnail
generation is unbounded background work triggered by directory navigation, it is
the single largest source of surprise CPU and IO in conventional file managers,
it wants a cache directory with its own eviction policy, and it interacts badly
with a zero-idle-CPU promise. As an *absolute*, it is weaker than it looks,
because the cost falls entirely on a group of users rather than on the system,
and because yazi already has the toggle: `mod+p` toggles preview, and image
preview could sit behind a second, off-by-default toggle without touching the
budget when it is off.

**Recommendation.** Keep "no thumbnails" as the default and the identity. Change
"ever" to "not by default": ship yazi with image preview disabled, document the
one-line change that enables it, and keep the promise the budget actually needs
— *nothing generates a thumbnail unless the user asked for it in this session*.
That preserves the cold-start number, preserves the aesthetic, and stops helm
from being the file manager someone cannot use for the one job they have.

This is a **design decision for a human**, not a research finding; see §19.

**Priority: n/a** (a decision, not a component). **Milestone: M4**, alongside the
charon portal work.

---

## 15. System updates and package management

**What it provides.** Knowing that updates are pending — urania's mockup says
`updates 3 held` — and applying them.

**Without it.** The user runs `apt`, `dnf` or `nixos-rebuild` themselves, which
is what a power-user desktop's audience does anyway.

**Must helm provide, integrate, or not break?** **Not break.** There is no
credible way for helm to own this across three package managers with three
different security models.

**The landscape, verified.** Fedora moved PackageKit to a DNF5 backend and
[Fedora 44 has GNOME Software talking to DNF5 directly](https://fedoraproject.org/wiki/Changes/PackageKit-DNF5),
with `dnf5daemon` as the D-Bus surface; PackageKit is being routed around rather
than extended. Ubuntu has `apt` plus `unattended-upgrades` and no D-Bus story
worth using. NixOS has no concept of a pending update at all — there is a
generation, and `nixos-rebuild` builds another one. A single cross-distro
"3 updates held" module means three implementations of three different
questions, one of which is not a question.

**Candidates.** `dnf5daemon` (Fedora, D-Bus), `apt list --upgradable` (Ubuntu,
a subprocess and a parse), `nvd`/`nix store diff-closures` (NixOS, and only
after a rebuild). PackageKit would have unified them and is going the wrong way.

**Fit.** Bad, for all of them. Any implementation is a subprocess on a timer —
the exact thing the bar's contract forbids — reporting a number the user cannot
act on from the bar anyway.

**Integration cost.** Real work for little value, or a shell hook the user
writes themselves.

**Priority: LATER**, and possibly never in the bar. If urania wants the line,
make it a user-supplied "spell" — a script whose output urania displays — rather
than a helm feature with three distro backends. **Milestone: M6.**

---

## 16. Media playback control — MPRIS and the media keys

Not in the numbered brief, and it belongs here: this is the "media" half of
"hardware, media and localisation", and it is the one thing in this document
that a user notices within an hour.

**What it provides.** Play/pause from the keyboard while the browser is not
focused; skipping a track; knowing what is playing.

**Without it.** The dedicated media keys on every laptop keyboard made since
2005 do nothing, and pausing music means finding the tab. Under river this is
guaranteed to be broken unless helm acts, for the same reason as §1 and §5:
helm owns every binding.

**Must helm provide, integrate, or not break?** **Provide the binding, integrate
with MPRIS.** MPRIS is a D-Bus specification every media player and browser
implements.

**Candidates.**

| Option | Language | Status (verified) | Notes |
|---|---|---|---|
| [`playerctl`](https://repology.org/project/playerctl/versions) | C | 2.4.1 on **every** target including Ubuntu 24.04 | The universal CLI. `playerctl play-pause` in the keymap is a one-line, zero-risk MVP answer |
| [`mpris` crate](https://crates.io/crates/mpris) | Rust, zbus-based | v2.1.0, 2026-04-18 | For a native now-playing bar module with `PropertiesChanged` push |
| [`mpris-server`](https://crates.io/crates/mpris-server) | Rust | v0.10.0, 2026-04-19 | Only relevant if helm ever *publishes* a player, which it will not |

**Idle inhibition belongs with this.** A video should not blank the screen.
river creates an idle-inhibit manager (`IdleInhibitManager.zig`, verified) and
an `ext-idle-notify-v1` notifier (`wlr.IdleNotifierV1` in `InputManager.zig`,
verified), so browsers and video players inhibit idling automatically and any
idle daemon helm ships gets proper notifications rather than polling for input.
The choice of idle daemon is the sibling file's.

**Fit.** Excellent and cheap. A now-playing module is not in the mockup's bar
and should not be added — the bar's centre is the window title, and the design
is emphatic about no wasted space. The keys are the whole feature.

**Integration cost.** Config-only for the keymap. A small Rust shim if a
now-playing readout is ever wanted (urania's almanac column would be the place,
not the bar).

**Priority: MVP** for the key bindings. **LATER** for any readout.
**Milestone: M3.**

---

## 17. A recurring pattern worth naming: hecate as the picker

Four of the components above want the same interaction: a filtered list of
things, chosen by keyboard, dismissed with escape. Wifi networks. Bluetooth
devices. Audio sinks. Display profiles. The off-the-shelf answers are four
different TUIs with four keymaps, four dependency stories and four packaging
problems (`impala`, `bluetui`, `wiremix` and `wlr-randr` are between them
missing from Debian, Ubuntu and Fedora in various combinations — all verified
above).

`helm-hecate` (M4) is already a fuzzy list over a source of items, driven by
`nucleo`. Making its item source pluggable — a trait with implementations for
PATH entries, desktop files, spells, wifi networks, Bluetooth devices and audio
sinks — turns four dependencies into four ~150-line providers over D-Bus
subscriptions helm already needs for the bar. The interaction is identical to
the launcher, so there is nothing new for the user to learn, and it is themed by
`palette.toml` directly rather than through a terminal's ANSI approximation.

That is a genuine architectural payoff and it should be considered when hecate
is specified, not retrofitted. It does not change the MVP: for M3, the TUIs in a
floating terminal are the correct stopgap, and they are swappable in exactly the
way ADR 0007's seam rule requires.

---

## 18. Summary table

| Component | Provide / integrate / not break | Recommended choice | Fit | Cost | Priority | Milestone |
|---|---|---|---|---|---|---|
| Audio volume and mute | integrate + provide the bindings | `pipewire` crate subscriber in `helm-session`; `wpctl` for actions | good — genuinely push-based | small Rust shim + keymap | **MVP** | M3 |
| Per-app audio routing | integrate | `wiremix`, `pulsemixer` fallback on Debian/Ubuntu | good — Rust TUI, ANSI-themed free | config-only | STRATEGIC | M4 |
| Wifi / wired / connectivity | integrate | NetworkManager + `nmtui`; hecate picker later | adequate; the VPN requirement decides it | config + Rust shim | **MVP** | M3 |
| VPN readout | integrate | NetworkManager `ActiveConnection` | good | Rust shim | STRATEGIC | M4 |
| Net throughput (`↑ ↓`) | provide | sampled `/proc/net/dev`, or move to horus | **poor — cannot be pushed** | bar module + a justified timer | STRATEGIC | M4 |
| Bluetooth | integrate | `bluetui`, `bluetoothctl` fallback | good shape, poor packaging | config-only | **MVP** | M3 |
| Output hotplug / workarea | provide | `river_output_v1` events in `RiverBackend` | perfect — protocol events | part of M2 | **MVP** | M2/M3 |
| Multi-monitor profiles | integrate | `kanshi` (shikane if packaging allows) | good; declarative config suits helm | config-only | LATER | M6 |
| Fractional scaling exactness | provide | extend the M2 seam test to 1.25/1.5/1.75 | risk to the central invariant | test work | **MVP** | M2 |
| Screen brightness | provide the binding | logind `SetBrightness` via zbus | excellent — no privilege, no polling | small Rust shim | **MVP** | M3 |
| Keyboard backlight | provide the binding | same call, `"leds"` subsystem | excellent | trivial once the above exists | STRATEGIC | M4 |
| External monitor brightness | integrate | `ddcutil`, opt-in | poor — slow, needs group membership | config + docs | LATER | M6 |
| Battery + warning level | integrate | UPower `DisplayDevice` over zbus | excellent — fully push-based | small Rust shim | **MVP** | M3 |
| Suspend / lid | not break | logind `HandleLidSwitch`; never hold a permanent inhibitor | excellent — nothing to build | config-only | **MVP** | M3 |
| Power profiles | integrate | `power-profiles-daemon` API (tuned-ppd satisfies it) | good — one API, both distros | Rust shim + `helmctl` verb | STRATEGIC | M4 |
| Keyboard layout | **provide** | implement `river-xkb-config-v1`; `XKB_DEFAULT_*` in the meantime | protocol is push-based and helm-shaped | **real work** | **MVP** | M2/M3 |
| Touchpad / pointer config | **provide** | implement `river-libinput-config-v1` | as above | **real work** | **MVP** | M2/M3 |
| Input methods / CJK | not break | fcitx5; river already serves `input-method-v2` **with popups** | good — better than sway | config-only + verification | LATER | M4 |
| Removable media | integrate | `udiskie --no-tray` as a user unit + yazi keymap | fine; Python is the only blemish | config-only | **MVP** | M3 |
| Printing | not break | CUPS as the distro ships it; document `localhost:631` | fine — nothing to do | docs | LATER | M6 |
| Night light | integrate | `wl-gammarelay-rs` (D-Bus), `wlsunset` fallback | very good — becomes a helm state | small Rust shim | STRATEGIC | M4 |
| ICC / HDR | not break | nothing — unreachable on river | n/a | none | LATER | M5 |
| Timezone / DST | integrate | `timedate1` `PropertiesChanged` + `jiff` | good — the signal exists | small Rust shim | **MVP** | M3 |
| `LANG` / `XKB_DEFAULT_*` import | provide | add to ADR 0011's two import lists | trivial and currently missing | config-only | **MVP** | M3 |
| Time sync | not break | timesyncd or chrony as shipped | fine | doctor line | LATER | M6 |
| Screen reader | not break + document | none exists on this stack | **none — no option at any price** | docs now, compositor work at M5 | **MVP** (the statement) | M3/M5 |
| Magnification | provide, eventually | impossible under river; `helm-compositor` requirement | blocked architecturally | M5 requirement | LATER | M5 |
| Sticky / slow keys | provide, eventually | same | blocked; helm may not even see raw keys | M5 requirement | LATER | M5 |
| Fonts / fallback chain | provide | generated `fonts.conf` from `typography.fallback` | perfect fit for the existing mechanism | one template | **MVP** | M3 |
| Thumbnails | decide | default off, one-line opt-in | see §14 | config-only | decision | M4 |
| System updates | not break | a user "spell", not a helm feature | poor for all candidates | none | LATER | M6 |
| Media keys (MPRIS) | provide the binding | `playerctl` in the keymap | excellent — one line | config-only | **MVP** | M3 |
| Idle inhibition while playing | not break | river's `IdleInhibitManager` handles it | good | none | **MVP** | M3 |

---

## 19. The event-driven audit

The bar's contract ([INTERFACES §3](../INTERFACES.md)) is "no timers except the
clock; a module that can only be polled must justify itself in its PR", and
ARCHITECTURE §4 puts bar idle CPU at approximately zero. This section takes that
seriously, because the mockup's own bar contains modules that cannot honour it.

### Genuinely push-based — no timer, no justification needed

| Signal | Mechanism | Verified? |
|---|---|---|
| Orbit, focus, layout, mode, window title | `river-window-management-v1` events | yes — the protocol XML |
| Workarea, output hotplug | `river_output_v1.position` / `dimensions` / `removed` | yes |
| Keyboard layout, caps lock, num lock | `river_xkb_config_v1.layout` / `capslock_enabled` events | yes — the protocol XML |
| Volume, mute, default sink | PipeWire `Node::subscribe_params` → `param` callback | yes — pipewire-rs docs |
| Battery percentage, state, warning level | UPower `PropertiesChanged` on `DisplayDevice` | assumed (long-standing interface) |
| AC/battery transition | same | assumed |
| Suspend / resume | logind `PrepareForSleep` | yes — login1 manual |
| Lock / unlock | logind `Lock`/`Unlock`; river's `session_locked` to the WM | yes |
| Network link up/down, SSID, VPN state | NetworkManager `PropertiesChanged` / `StateChanged`; netlink `RTM_NEWLINK` for the low-level view | assumed |
| Bluetooth device connect/disconnect | BlueZ `ObjectManager` `InterfacesAdded` + `PropertiesChanged` | assumed |
| Removable device appeared | udisks2 `ObjectManager` `InterfacesAdded` | assumed |
| Timezone changed | `timedate1` `PropertiesChanged` on `Timezone` | yes — timedate1 manual |
| Backlight changed by firmware | udev `change` on the `backlight` subsystem | **assumed — test on real hardware** |
| Idle / active | `ext-idle-notify-v1` (river creates `wlr.IdleNotifierV1`) | yes — river source |
| Theme reload | helm's own fan-out | n/a |

### Cannot be pushed — the bar's design has a problem here

| Signal | Why it cannot be pushed | Where it appears |
|---|---|---|
| **CPU utilisation** | `/proc/stat` is a set of counters. Utilisation is a *rate*: two samples, subtracted. The kernel has no "CPU is now at 31%" event and cannot have one | bar `cpu 31%`, horus, urania `31% load` |
| **Memory used** | `/proc/meminfo` is a snapshot with no notification. **Partial exception:** [PSI](https://docs.kernel.org/accounting/psi.html) triggers written to `/proc/pressure/memory` are `poll()`-able and wake on a stall threshold — genuinely event-driven, but they report *pressure*, not `9.8G`, and unprivileged windows must be a multiple of 2 s (verified) | bar `mem 9.8G` |
| **Network throughput** | `↑ 18k ↓ 1.2M` is a rate over byte counters. netlink pushes *link state* changes, never counter updates | bar net module, horus sparklines |
| **GPU / CPU temperature** | hwmon sysfs is read-on-demand. **Partial exception:** the kernel's [thermal generic-netlink interface](https://lwn.net/Articles/986009/) pushes trip-point and threshold crossings, and thresholds are settable from userspace — so "too hot" is an event, while "44°" is not (verified that the mechanism exists; not verified that it covers consumer GPU sensors) | bar `gpu 44°`, urania `44° gpu`, horus |
| **Entropy pool, ping, disk usage** | all sampled | urania readouts |

### What this means, stated plainly

Four of the seven right-hand bar modules the design specifies — cpu, mem, gpu
and the throughput half of net — **cannot be event-driven**. This is not a
missing library or a lazy implementation; the kernel does not expose these as
events, and no status bar on any platform gets them any other way. `waybar`,
`i3status-rs` and every other bar sample them on an interval.

So the rule as written ("no timers except the clock") cannot survive contact
with the mockup. There are four honest options:

1. **Accept one sampling timer, deliberately.** A single shared sampler in
   `helm-session` at a stated interval (2 s is the usual choice; 5 s is
   defensible and halves the wakeups), producing one `HelmState` update that
   coalesces cpu, mem, gpu and net rates. One timer for the whole desktop,
   not four. The bar still redraws only when a value *changed*, so
   `renders_same_as` still does its job, and `mem 9.8G` changing at 5 s
   granularity is invisible to a human.
2. **Move the sampled values out of the bar and into horus.** btop is already
   the monitor, it is already sampling, and it is only on screen when the user
   asked for it. The bar keeps orbit, layout, mode, title, volume, battery and
   clock — every one of which is push-based — and drops cpu/mem/gpu/net.
   Idle CPU genuinely reaches zero. The cost is a visibly emptier bar than the
   mockup, which is high-fidelity and "final unless noted".
3. **Sample only when visible and only when changed.** Keep the timer but stop
   it when the bar is occluded by a fullscreen window, and back it off on
   battery. Complexity for a modest win.
4. **PSI and thermal thresholds instead of numbers.** Replace `cpu 31%` with a
   pressure indicator that is genuinely event-driven, and `gpu 44°` with a
   threshold-crossing warning. Honest, cheap, and a different design than the
   one that was approved.

**Recommendation: option 1, with option 2 as the fallback if measurement shows
the timer costs more than it earns.** Option 1 should be written up as an ADR
amending ADR 0009's "no timers" language, because a rule the shipped design
cannot follow is worse than a rule with one documented exception. The ADR should
state the interval, state that there is exactly one sampler process-wide, and
require that the sampler runs off the window-management event loop for the
liveness reason in §0.2.

Two smaller consequences fall out of the same audit:

- **`gpu 44°` is the worst offender per unit of value.** It needs hwmon paths
  that differ per vendor, or NVML for NVIDIA, it is the least actionable number
  on the bar, and it is duplicated in horus and urania. It is the first thing to
  cut if the bar needs to lose a module.
- **The clock's 1 s tick is fine and should stay.** It damages one text run,
  ARCHITECTURE §4 already budgets for it, and `jiff` can compute the exact
  duration to the next minute boundary so the tick can be once a minute for the
  displayed format — the mockup shows `14:32`, not seconds.

---

## 20. The uncomfortable list

Where the best available option fits helm's vision badly and there is no good
answer.

1. **Accessibility, and specifically the screen reader.** There is no option.
   Not a bad option — none. A blind user cannot use helm, cannot use sway,
   cannot use river, and this will not change until Newton ships and a
   compositor implements it. helm's honest positions are to document it
   prominently and to make `helm-compositor` a place where it becomes possible.
   Anything else is marketing.
2. **Sticky keys versus a chorded keymap.** helm's core interaction is a
   modifier chord. The accessibility feature that makes chords usable one-handed
   is the one feature helm cannot implement on river, and possibly cannot
   implement even in principle given that `river-xkb-bindings-v1` delivers bound
   keys rather than raw ones. A keyboard-first desktop owes this more than a
   mouse-first one does.
3. **Four bar modules cannot honour the bar's own contract.** §19. The design is
   high-fidelity and the rule is a gate rather than a goal; one of them has to
   move, and pretending otherwise means the rule quietly becomes advisory —
   which is how frame budgets die.
4. **The best-fitting tools are the worst-packaged.** `wiremix`, `impala`,
   `bluetui`, `shikane` and `wl-gammarelay-rs` are the five candidates that fit
   helm best on every axis that matters — Rust, keyboard-driven, themeable
   through the ANSI palette, no toolkit. Between them they are absent from
   Debian, Ubuntu and Fedora in almost every combination (verified). The
   well-packaged alternatives (`pavucontrol`, `blueman`, `nmtui`,
   `nm-connection-editor`, `gammastep`) are mostly GTK, mouse-shaped, or both.
   helm can build them from crates.io in its own packages, which means owning
   five more builds; or ship worse defaults on two of three targets; or write
   its own (§17). None of the three is comfortable.
5. **udiskie is Python.** In a Rust-first desktop, the automounter is a Python
   daemon. The alternative is writing one, which ADR 0007 says not to do without
   a written reason, and "it is not Rust" is not a written reason.
6. **NetworkManager is a large, opinionated daemon in a minimal desktop.** iwd
   is what helm would choose on taste. VPN is what makes the choice for us, and
   the design put `vpn ◉ warded` in the mockup.
7. **HDR is unreachable and will be asked about.** river creates the
   colour-management global only on a capable renderer and exposes no way to
   configure output colour (verified). Users with HDR monitors will ask, and the
   answer until M5 is "no".
8. **"No thumbnails, ever" is a promise made on behalf of users who are not in
   the room.** §14.

---

## 21. What helm should build itself

[ADR 0007](../adr/0007-reuse-yazi-btop-starship.md) demands a written reason for
anything helm writes rather than reuses. Four items qualify.

### 21.1 The hardware state layer in `helm-session` (not a status bar)

**Reason.** Every off-the-shelf status bar — waybar, i3status-rs, eww — polls on
an interval, ships its own theming language, and owns its own process. helm
already has a state broadcaster (ADR 0003), a wire format (ADR 0004), a palette
(ADR 0005) and a bar (ADR 0008). Adding a second state-owning process to feed
the first would create the second source of truth ADR 0001 exists to prevent.
The build is a set of subscribers — zbus for UPower, logind, NetworkManager,
timedated; the `pipewire` crate for audio — folded into `HelmState`, on a
thread that never touches the window-management event loop.

**Not built:** the daemons themselves. helm subscribes; it does not implement a
battery monitor, a network manager or an audio server.

### 21.2 The two river configuration protocols

**Reason.** Not a choice. Under river 0.4 the window manager is the input
configuration; there is nothing to reuse because there is no other client
allowed to do it (river sends `unavailable` to a second window-management
client). §0.1.

### 21.3 A generated `fonts.conf`

**Reason.** The alternative is the fallback chain being written down twice — in
`palette.toml` for helm's own renderer and in a hand-maintained fontconfig file
for everything else — which is precisely what ADR 0005 forbids. §13.

### 21.4 Picker providers for hecate, later

**Reason.** helm is already building hecate for other reasons; four TUI
dependencies that fit badly and package worse can be replaced by four small
providers over D-Bus connections helm already holds. This is reuse of helm's own
component rather than a rewrite of anyone else's: `impala` and `bluetui` stay
supported for people who prefer them. §17. **Not before M4**, and not at the
cost of M3.

### What helm should *not* build

An audio server, a network manager, a Bluetooth stack, an automounter (§20.5), a
night-light daemon, a display-configuration tool, a printing system, a screen
reader, or a package manager front end. Every one of those is a multi-year
project whose first two years would be worse than what exists.

---

## 22. Needs a human

Flagged with options and a recommendation, per standing order S3.

**A. ADR 0013's protocol table is missing two protocols (§0.1).**
Options: (a) amend ADR 0013 and add both to M2's scope; (b) implement only
`river-xkb-config-v1` at M2 and defer libinput configuration to M4, shipping an
MVP where laptop touchpads have no tap-to-click; (c) treat both as M4 and
document the limitation.
**Recommendation: (a).** A laptop without tap-to-click will not survive the week
test, and the protocols are declarative setters rather than deep work.

**B. The bar's four unpushable modules (§19).**
Options: (1) one shared sampler at a stated interval, ADR-documented as the
single exception; (2) move cpu/mem/gpu/net into horus and leave the bar
push-only; (3) sample only when visible; (4) replace numbers with PSI and
thermal thresholds.
**Recommendation: (1), with (2) as the fallback.** This one changes a shipped
rule and a high-fidelity design, so it should not be settled by an
implementation choice in a PR.

**C. "No thumbnails, ever" (§14).**
Options: (a) keep "ever" as an absolute; (b) default off with a documented
one-line opt-in; (c) default off with a helm-level toggle in charon's config.
**Recommendation: (b).** It costs nothing, keeps every budget, and stops helm
from being unusable for one common job.

**D. Packaging the well-fitting Rust tools (§20.4).**
Options: (a) build `wiremix`, `impala`, `bluetui` and `wl-gammarelay-rs` from
crates.io inside helm's `.deb` and `.rpm`; (b) ship `pulsemixer`, `nmtui`,
`bluetoothctl` and `wlsunset` on Debian/Ubuntu and the better tools on Arch and
Nix, accepting an inconsistent desktop; (c) accept the distro-packaged options
everywhere and lose the fit.
**Recommendation: (a) for `wiremix` and `bluetui`** (small, pure Rust, already
in the build toolchain), **(b) for the rest**, revisited when hecate's providers
land.

**E. What a low-battery warning looks like (§6).**
Options: (a) recolour the bar's battery module through `palette.toml` at
`WarningLevel = Low`, and again at `Critical`; (b) a notification through
whatever daemon the sibling file selects; (c) both, with (a) at Low and (b) at
Critical.
**Recommendation: (c).** Colour is free and matches the design's restraint;
`Action` level deserves something a user cannot miss.

**F. Accessibility posture (§12, §20.1–2).**
Options: (a) say nothing and let users discover it; (b) a clear statement in
INSTALL and a `doctor` line; (c) (b) plus a written commitment that
`helm-compositor` will support magnification and AccessX, recorded as a
requirement in M5's spec before M5 begins.
**Recommendation: (c).** (a) is not acceptable; (b) alone leaves the situation
permanent by default.

---

## 23. Verification ledger

**Verified during this research** (read directly, with links above): river's
protocol directory contents and the two undocumented protocols; the
`river_output_v1` interface's complete request and event list; river's default
keymap coming from `XKB_DEFAULT_*`; river's creation of `wlr.OutputManagerV1`,
`wlr.GammaControlManagerV1`, `wlr.OutputPowerManagerV1`, `wlr.ScreencopyManagerV1`,
`wlr.IdleNotifierV1`, `wlr.InputMethodManagerV2`, `wlr.TextInputManagerV3`,
`wlr.PointerConstraintsV1`, `wlr.TabletManagerV2` and the conditional
`wlr.ColorManagerV1`; the absence of HDR/ICC handling in river's `Output.zig`;
river's 1/120 scale rounding; the absence of any accessibility protocol in
`wayland-protocols/staging`; `logind.SetBrightness`'s interface, signature and
privilege requirements; `timedate1`'s property change-signal annotations;
`Node::subscribe_params` in pipewire-rs; crates.io versions and dates for
`pipewire`, `zbus`, `udisks2`, `brightness`, `shikane`, `wiremix`, `impala`,
`bluetui`, `jiff`, `mpris`, `sunrise` and `wireplumber`; distribution versions
for every tool named, via repology; udiskie 2.7.0's release date via PyPI;
libcups 3.0.0's release date; PSI trigger semantics; the existence of thermal
netlink threshold events; yazi's image-preview protocol support.

**Also verified, and relevant to a decision this document does not own:** no
Debian source package named `river` exists, and repology lists river 0.4 in no
distribution at all — only `river-classic` 0.3.x in Arch, Fedora and nixpkgs.
That is stronger than ADR 0013's stated assumption that "Ubuntu and Fedora ship
river 0.3.x or river-classic", and it strengthens the case for vendoring.

**Assumed, not verified — treat as work items:**

- The exact NetworkManager and BlueZ D-Bus signals named in §19 (long-standing,
  stable interfaces, but I did not re-read the specifications).
- libinput's per-device default for tap-to-click, which comes from its quirks
  database rather than from a single global default.
- That backlight changes made by firmware raise udev `change` events on the
  `backlight` subsystem. This needs testing on real hardware before any bar
  module depends on it.
- That `nmtui` is present in the default NetworkManager package on all three
  targets. Confirm in packaging CI.
- That CJK input works end to end in foot under river with fcitx5. The
  mechanism is verified to exist on both sides; the integration is not tested.
- That river implements no AccessX (sticky/slow/bounce keys). I found no
  evidence of it and did not read `Keyboard.zig` in full.
- Whether thermal netlink threshold events cover consumer GPU sensors, as
  opposed to CPU thermal zones.
- Whether `river-xkb-bindings-v1` could expose a raw key stream sufficient for
  helm to implement sticky keys itself. This determines whether §20.2 is
  permanent or merely current.
