# The integration surface

A desktop environment is not a compositor and a bar. It is expected to provide
or integrate with several dozen OS-level services, and users notice within
minutes when one is missing — usually as "this is broken", not as "this feature
is absent".

This is the prioritised register. The evidence sits in two research documents
and is not repeated here:

- [`integration/session-services.md`](integration/session-services.md) — notifications, lock and idle, polkit, clipboard, screenshot, portals, secrets, MIME and autostart, tray, session control, MPRIS
- [`integration/hardware-media.md`](integration/hardware-media.md) — audio, network, bluetooth, displays, brightness, power, input methods, removable media, printing, colour, locale, accessibility, fonts

**Priority means one thing only:** `MVP` is "a person cannot use helm as their
only desktop for a week without it" ([MVP.md](MVP.md)). Everything else is
`STRATEGIC` or `LATER`, however much it matters in the abstract.

---

## 1. MVP — the week test (M3 unless noted)

Ordered by what breaks first if it is missing.

| # | Capability | Recommended | Cost | Without it |
|---|---|---|---|---|
| 1 | **Portals: FileChooser, ScreenCast, Settings, `Inhibit=none`** | xdp-gtk + xdp-wlr, named explicitly | config | Firefox cannot upload a file; screen sharing fails silently; every GTK4 and Flatpak app renders **light** on a black desktop; the screen blanks mid-call |
| 2 | **Notifications** | `fnott` (fallback `mako`) | config + template | Slack, Element and calendar reminders are silent; some clients hang on the D-Bus call |
| 3 | **Lock and idle** | an `ext-session-lock-v1` client + `swayidle` | config | A laptop lid closes onto an unlocked desktop. **`needs-human`** — see §4 |
| 4 | **Polkit agent** | `soteria` | config | "Not authorized" when mounting a USB stick, with no prompt to answer |
| 5 | **Wifi and VPN** | NetworkManager + `nmtui` | config + binding | Cannot join a network without dropping to a terminal |
| 6 | **Audio volume and mute** | PipeWire subscriber + `XF86Audio*` bindings | shim | **Under river, helm owns every binding — if helm does not bind the volume key, nothing does** |
| 7 | **Brightness, battery, lid** | logind `SetBrightness`, UPower `WarningLevel` | shim | No brightness control; no low-battery warning; lid close does nothing |
| 8 | **Keyboard layout and touchpad** (**M2**) | serve `river-xkb-config-v1` + `river-libinput-config-v1` | real work | **A laptop has no tap-to-click and no way to get one**; layouts frozen at whatever river started with |
| 9 | **Clipboard survival** | `wl-clip-persist` | config | Copy, close the window, paste nothing |
| 10 | **Secrets** | `gnome-keyring` | config | Every app that stores a password prompts on every launch |
| 11 | **Locale and XKB in the import set** | add `LANG`, `LC_*`, `XKB_DEFAULT_*` to ADR 0011's lists | trivial | Apps disagree about date format and sort order, unattributably |
| 12 | **`fonts.conf` generated from `typography.fallback`** | one template | one template | The glyph contract is written down twice and only helm's own renderer honours it |
| 13 | **MIME, autostart, `xdg-user-dirs`** | shipped config | config | Downloads land in `$HOME`; "open with" does nothing |
| 14 | **Screenshot** | `grim` + `slurp` | config + binding | No screenshots |
| 15 | **Media keys** | `playerctl` in the keymap | one line | Play/pause does nothing |
| 16 | **Removable media** | `udiskie --no-tray` + a charon keymap entry | config | Plugging in a USB stick does nothing visible |
| 17 | **Session control** | build `helmctl session {logout,reboot,poweroff,suspend,lock}` | ~150 lines | No way to log out except killing the session |
| 18 | **`xdg-activation` token handling** (**M2**) | `RiverBackend` | shim | "Open link" focuses the wrong window |
| 19 | **The honest accessibility statement** | INSTALL + `doctor` | docs | We would be claiming a desktop we have not built. See §5 |

## 2. Strategic — after the MVP (M4)

Bluetooth (`bluetui`) · MPRIS beyond the keys · clipboard history as a hecate
source · a tray decision (§4) · night light (`wlsunset`) · multi-monitor
profiles (`shikane`) · the thumbnails decision · input methods for CJK, whose
mechanism is verified present in river but whose end-to-end path is untested.

## 3. Later (M5–M6)

Printing — nothing to do, all three targets still ship CUPS 2.4.x · system
updates, which belong in a user "spell" rather than in helm · ICC and HDR,
unreachable on river today · external-monitor brightness · desktop search,
where the recommendation is to ship none and say so · time sync.

---

## 4. What helm builds rather than reuses

Each needs the written reason [ADR 0007](adr/0007-reuse-yazi-btop-starship.md)
demands.

| Build | Reason |
|---|---|
| `helmctl session …` | `wlogout` and `wleave` are icon-button grids in GTK. The which-key strip already *is* the right interface, and ~150 lines of `zbus` against logind is cheaper than integrating a toolkit |
| Volume and brightness feedback in `helm-bar` | Every OSD daemon's differentiator is the fade, which ADR 0009 forbids. The design already has `♪ 64%` in the bar |
| Clipboard-history recall as a hecate source | Reuse the store, not the picker — every manager delegates to fuzzel or rofi anyway |

Explicitly **not** built: notification daemon, lock screen, secrets daemon,
portal backend. Those are security-relevant surfaces where being early with a
user's passwords is a different class of risk from being early with a bar.

## 5. The uncomfortable list

Where the best available option fits helm badly and there is no good answer.
These are `needs-human`, and pretending otherwise would be the failure mode
[S15](../.claude/memory/00-standing-orders.md) exists to prevent.

| Problem | Why it is uncomfortable |
|---|---|
| **Accessibility** | There is no accessibility protocol in `wayland-protocols/staging` (verified by listing it), Orca does not work on wlroots compositors, and `tiny-skia` + `cosmic-text` expose no a11y tree. Worse for helm specifically: **magnification is architecturally impossible before M5** — `river_output_v1` has two requests and neither is zoom — and **sticky keys cannot be implemented at all**, because `river-xkb-bindings-v1` delivers *bound* keys rather than the raw stream. Sticky keys is what makes a chorded-modifier desktop usable one-handed. A keyboard-first DE owes that more than a mouse-first one does. The free wins are real (no motion, derived contrast, full keyboard reachability) and they do not cover a blind user |
| **Tray / SNI** | An icon protocol with mouse menus, against a design with no tray region and a no-icon-scan cold-start budget. But Slack, Element, Steam, Nextcloud and KeePassXC lose their "close to tray" recovery path. Three options, none clean |
| **Polkit agent** | Every maintained agent is a toolkit modal. The least helm-looking surface in the whole desktop will be the privileged password prompt |
| **Lock screen** | `waylock` fits (four command-line colours, same maintainer as river) but has no recent tag; `gtklock` themes from the `gtk.css` helm already generates but puts GTK3 in the lock path. The trade is attack surface against visual fidelity on the one surface between a locked laptop and its contents |
| **Secrets** | Rust-first here means being early with the user's passwords |
| **The best-fitting tools are the worst-packaged** | `wiremix`, `impala`, `bluetui`, `shikane`, `wl-gammarelay-rs` fit helm best and are absent from Debian, Ubuntu and Fedora in nearly every combination. The well-packaged alternatives are GTK and mouse-shaped |
| **Notification glyphs** | Neither daemon is covered by [ADR 0012](adr/0012-font-fallback-is-a-contract.md)'s glyph contract, so a rune can render as tofu in a notification even though it cannot in the bar |
| **"No thumbnails, ever"** | A promise made on behalf of users not in the room. yazi already does Sixel preview in foot, so the feature is being switched *off*, not skipped. The recommendation is "not by default" rather than "ever" |

## 6. The finding that changed the design

Four of the mockup's seven bar modules **cannot** be event-driven. `cpu`, `mem`,
`gpu` temperature and the `↑ 18k ↓ 1.2M` half of `net` are rates over counters:
`/proc/stat` and `/proc/meminfo` have no notification mechanism, and the kernel
cannot have one. Partial exceptions exist and were checked — PSI triggers on
`/proc/pressure/*` are genuinely `poll()`-able, and thermal netlink pushes
threshold crossings — but they report *pressure* and *too hot*, not `9.8G` and
`44°`.

Everything else on the bar is genuinely push-based, verified case by case:
orbit, focus, mode, title, volume, battery, layout, VPN, clock.

So "no timers except the clock" could not survive the design it served. One
shared sampler now lives in `helm-session`, off river's input path, as the
single documented exception; the bar owns no timer and remains a pure function
of `HelmState`, which is the property that actually mattered. See
[ARCHITECTURE §4](ARCHITECTURE.md) and [INTERFACES §3](INTERFACES.md).

## 7. Everything here is gated on one thing

Every layer-shell client above — the notification daemon, the lock screen, the
launcher, helm's own bar — appears **only if `helm-session` serves
`river-layer-shell-v1`**. Under river, layer-shell is the window manager's job.
Until that M2 work lands, none of these recommendations has been run, and when
one of them fails it will look like the client crashed rather than like helm
never offered it.

Nothing in this register has been tested on hardware.
