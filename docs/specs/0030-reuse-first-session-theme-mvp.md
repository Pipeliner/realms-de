# SPEC 0030 — Reuse-first MVP: session themes and bar comparison

- **Status:** Accepted (owner approval, 2026-09-27); implementation pending
- **Milestones:** M1–M3
- **Decision:** [ADR 0023](../adr/0023-reuse-first-session-theme-mvp.md)

## Authority and preserved behavior

This specification controls conflicting MVP theme selection/launch-ownership
clauses in SPECs 0002/0003/0005/0006/0011/0012/0013/0020 and INTERFACES.md.
It controls renderer-choice clauses in SPEC 0004 and ADR 0008. Other clauses
remain applicable: palette format/lint, window semantics, control transport,
user-file preservation, coherent assets, portal routing, packaging and lock/idle.
The historical generation specifications are shelved as product obligations,
not evidence that their current implementation may be bypassed unsafely.

## T1 — Theme selection and publication

`realmctl theme apply` validates and renders a complete set of supported outputs
from the selected palette and prepares it for the **next graphical login**.
It reports that scope explicitly. Validation/render/publication failure leaves
the previously prepared complete selection available; never select a partial
output set. Existing reusable staging/validation code may remain an internal
implementation detail. Success is not a claim that any live process changed.

At login, resolve one complete prepared selection before starting themed Realm
clients and use it for that session. If no selection exists, prepare the shipped
default with the same validation. A corrupt selection is diagnosed, not silently
replaced by guessed data. Later apply operations must not mutate the selected
session assets or environment. MVP permits one active Realm session per user
(owner confirmed 2026-09-27). Reject a competing same-UID Realm login without
disturbing the existing session. Sessions belonging to different users keep
independent theme selections. Persist only the lifetime information needed.

The session must not reclaim configuration still in use. For MVP, leaving old
published theme data intact is acceptable; automatic generation GC and bounded
retention are not launch gates. Do not delete legacy leases or protected trees
as part of switching launch semantics. A later cleanup change needs its own
ownership proof. This exception concerns theme data, not the separate required
bounded Cargo/build-cache cleanup system.

### Login-selection implementation boundary

Reuse the generation store's complete validation and process-identity leases.
A lookup by the already-selected generation ID SHALL validate and lease that
generation without consulting or changing the next-login `current` pointer.
Missing or corrupt selected data fails rather than falling back to `current`.
A login helper may explicitly retain its existing lease for the graphical
entry process after successful handoff. This consumes the helper's selection
and closes its descriptors without removing the lease. Normal drop still
releases an unhanded-off lease. Existing live-process identity checks protect
the retained generation until the graphical entry exits; existing stale-lease
reconciliation remains usable. No new lifecycle manager is required.

The graphical entry publishes `realm/session-theme.json` under its existing
per-user runtime directory before starting the compositor. The record contains
the absolute configuration root, immutable generation ID, graphical-entry PID,
Linux process start time and boot ID. Publication is atomic; a live existing
owner cannot be replaced. A new login may replace a well-formed dead-owner
record after taking the exclusive login claim. Malformed records fail with a
diagnostic. Consumers validate the owner identity and selected generation; a
missing or stale record is an error, never a request to select `current`.
The helper retains the owner's process lease only after successful publication.
WM restarts read this record instead of bootstrapping a new selection.
The record is private runtime state, not global toolkit activation environment.
The entry invokes the private `realm-wm --prepare-session-theme PID` command
with its own PID; failure aborts before compositor startup. The existing
explicit no-WM diagnostic mode remains available when the WM is absent.
The graphical entry takes a nonblocking exclusive `flock` on
`$XDG_RUNTIME_DIR/realm-session.lock` before compositor/PID/environment changes.
The entry owns the descriptor directly, preserving its signal/teardown identity;
compositor and direct-client subprocesses close their inherited copy before
exec. Contention exits 73 without running session cleanup. The lock file is not
unlinked on exit; entry exit releases the claim after teardown. Native packages
declare the flock executable dependency and the Nix wrapper supplies it.

The published generation includes its validated source palette bytes as the
manifest-listed `realm/palette.toml` output. The bar reads this immutable output
through the same live-login selection as other clients, including after a bar
restart; it does not reread user, system or compiled fallback palettes. Shared
login-record loading belongs in the theme library rather than making the bar
depend on the WM/session implementation. A previously prepared generation
without this required output is incomplete for this contract: diagnose it and
require an explicit `realmctl theme apply` before login, rather than borrowing
mutable palette bytes or silently changing the selection. This repository has
no supported pre-MVP user migration obligation.
Explicit apply publishes a new complete generation even when palette/template
inputs match an older snapshot-less generation; immutable old data is never
overwritten to retrofit the snapshot. Existing random generation IDs already
permit this; no new identity mechanism is needed.

Verification covers login A, source edits and apply B, bar restart still using
A, and next login using B. Missing/corrupt selected palette data must fail
without falling back. Publication/diff tests include the palette snapshot in
the complete output set.

## T2 — Consumer activation

Configure foot, fuzzel, Yazi, btop, zsh/Starship, GTK and Qt using their supported
configuration/environment/argument mechanisms, selected at login. Retain useful
existing templates and adapters. Preserve existing user files and avoid changing
another desktop's defaults. A session-local config overlay may be used where a
tool cannot import a theme. Do not claim GTK/Qt coverage without real packaged
consumer evidence, including supported runtime/plugin versions.

GTK3 and GTK4 `font-family` output must serialize each ordered
`typography.fallback` family as a quoted CSS string. Escape quotes,
backslashes and control characters within each family; joining raw palette
names is invalid for a name such as `Noto Sans Symbols 2`. The selected font
order and other template consumers must not change. A source-only parse of
the previously rendered GTK3 sheet with GTK 3.24.33 reported line 49
`Junk at end of value for font-family`; the exact GTK 3.24.52 VM stderr still
requires CI evidence, and this parser result alone does not complete V4.

The packaged toolkit VM probe must distinguish compositor-reported focused
window title from visible text. It must require exactly two managed windows
(the terminal and launched toolkit in this fixture), focus on that toolkit's
observed protocol title,
and independent OCR of its visible label before screenshot, file-open trace,
diagnostic, and clean-exit checks. On the pinned GTK3 widget factory the
observed River title is `gtk3-widget-factory`. Both direct and launcher probes
must independently recognize the visible `togglebutton` widget label, not a
copy of the protocol title or a terminal command. CI run 36354366061 showed
that OCR repeatedly read the narrow `Page 1` tab as `Pagel` or `Pace 1l`,
while reading the actual `togglebutton` control consistently. Changing that
single GTK3 visual anchor must not loosen the exact focused-title and two-window
checks, screenshot, selected CSS file-open proof, strict diagnostics, or clean
exit. A title mismatch must not be excused by OCR, or vice versa.
For the pinned GTK4 widget factory, the protocol title is `GTK Widget Factory`
and visible stack-switcher text is `Page 1`; Qt6ct uses `Qt6 Configuration Tool`
for both. These title expectations are grounded in the packaged upstream
sources: [GTK3 title propagation](https://github.com/GNOME/gtk/blob/3.24.52/gtk/gtkwindow.c#L4199),
[GTK3 Wayland fallback](https://github.com/GNOME/gtk/blob/3.24.52/gdk/wayland/gdkwindow-wayland.c#L484),
[GTK3 togglebutton widget](https://github.com/GNOME/gtk/blob/3.24.52/demos/widget-factory/widget-factory.ui#L908),
[GTK4 UI and stack switcher](https://github.com/GNOME/gtk/blob/4.22.4/demos/widget-factory/widget-factory.ui#L428),
and [Qt6ct UI title](https://www.opencode.net/trialuser/qt6ct/-/raw/0.11/src/qt6ct/mainwindow.ui).

When a packaged toolkit emits CSS/theme diagnostics, the VM must log bounded
matching stderr and bounded openat lines for its required theme files before
rejecting the probe. The diagnostic rejection remains strict; a grep match
cannot be treated as a consumer success. The GTK3 diagnostic text in CI run
36350431294 was not retained, so that run does not establish a CSS root cause
or justify relaxing this gate.

On the bare NixOS Realm session, GTK3's bundled Adwaita assets require an SVG
GdkPixbuf loader even when Realm's own CSS parses. CI run 36358017601 emitted
`Could not load a pixbuf from /org/gtk/libgtk/theme/Adwaita/assets/bullet-symbolic.svg`.
Register the pinned librsvg loader through NixOS's gdk-pixbuf module and pass
its generated loader-cache path to the Realm WM service, which launches the
terminal and toolkit consumers. The running WM and selected terminal must
inherit the same cache path, and that cache must actually list the SVG loader;
an environment variable alone is insufficient. Do not expand SPEC 0005's
shared systemd/D-Bus session-import allowlist or silently drop the real GTK3
warning check. The toolkit VM must continue to prove an actual open of the
selected generation CSS and log bounded relevant trace and selected GTK
environment evidence before a diagnostic failure so an absent open can be
distinguished from a loader failure. The missing selected CSS open in that CI
run remains unproven until the next runtime trace.

For pinned librsvg 2.62.3, the loader-cache assertion must match the actual
`libpixbufloader_svg.so` output of its
[Meson target](https://github.com/GNOME/librsvg/blob/2.62.3/gdk-pixbuf-loader/meson.build#L8-L14),
not the older hyphenated filename. If that exact entry is absent, retain a
bounded excerpt of the cache before failing; neither cache existence nor an
environment variable alone proves SVG support. CI run 36361669976 stopped at
the obsolete filename assertion before GTK launched, so GTK warning removal
and selected-CSS consumption remain unverified until another VM run.

The shared Fuzzel template must parse on Ubuntu 24.04's supported Fuzzel 1.9.2
as well as newer Fedora/Nix versions. Use the common configuration vocabulary:
`[colors]` contains background, text, match, selection, selection-text,
selection-match and border; omit newer prompt, placeholder, input and counter
color keys. The supported `[main]` prompt string remains configured. Source
rendering tests enforce this compatibility floor; native launcher roundtrips
prove actual installed parsing and usable launch, including after unlocking.
Reference: [Ubuntu's Fuzzel 1.9.2 manual](https://manpages.ubuntu.com/manpages/noble/man5/fuzzel.ini.5.html).

Applications launched from the session inherit its selected configuration;
standard desktop launchers and D-Bus activation are allowed. Already-running
single-instance applications or services outside that session may retain their
old appearance. Document this boundary and do not globally overwrite theme
environment to retheme unrelated sessions. No exact process ancestry, fresh-Exec
only admission, per-launch seal verification, generation lease transfer or
descendant-drain witness is part of MVP acceptance. Existing browser default
selection in SPEC 0027 is unchanged.

Session shutdown uses ordinary service/scope lifecycle management. Required
environment discovery, portal startup ordering and client isolation remain;
UWSM is an optional implementation evaluation, not a new prerequisite.

### Reused consumer integration boundary

Reuse the terminal/toolkit assets and probes from PRs #240/#241, but select the
login record rather than `current`. The terminal executes zsh with the generated
profile, Starship configuration and Yazi configuration/keymap. Its btop adapter
passes the separate supported config and themes-directory arguments; the
published btop config is read-only so ordinary writeback cannot alter the
session snapshot. No additional garbage-collection policy is introduced.

Foot configuration selection probes the generated modern and legacy section
variants with `--check-config` under one shared one-second deadline. It never
falls back to a mutable user configuration after both variants fail.
Ubuntu 24.04's Foot 1.16.2 requires cursor colors as `color=` under `[cursor]`;
its `[colors]` section does not accept `cursor=`. Keep the legacy generated
variant compatible with that installed parser while the modern variant retains
its supported `[colors-dark]` form. Both cursor colors remain palette-derived.
Reference: [Ubuntu's Foot 1.16.2 manual](https://manpages.ubuntu.com/manpages/noble/man5/foot.ini.5.html).
Hung-command fixtures must actually remain blocked with their restricted PATH
on native and Nix test hosts. Use a shell-builtin self-stop, not a host-specific
`/bin/sleep` path, and retain stderr when the expected timeout is absent.
The standalone shell-behaviour check invokes the fixture through its declared
shell interpreter, including with a writable fixture descriptor still open;
Linux executable-file exclusion must not obscure the self-stop assertion.
Real consumer tests continue executing the fixture directly through production
launch/probe code.

One generation-derived child environment supplies REALM_GENERATION, ZDOTDIR,
STARSHIP_CONFIG, YAZI_CONFIG_HOME, GTK_THEME and the Qt platform selector. GTK
uses the generated named-theme aliases; Qt uses the generated qt6ct profile.
The generated `ZDOTDIR` is a sealed output tree, never a cache directory.
Ubuntu's global interactive Zsh startup invokes `compinit` before `.zshrc` and
otherwise creates `.zcompdump` under `ZDOTDIR`. The generated `.zshenv` must
suppress only that global invocation before it runs; the generated `.zshrc`
must initialize completion itself with a dump under the user's writable XDG
cache (or without a dump if the cache directory or an existing dump is not
writable). Completion remains
available, and no Zsh startup writes into the sealed generation. An unlisted
generation entry still fails validation; its diagnostic names only the escaped
relative entry so CI can identify the writer without accepting that mutation.
The installed native terminal probe records the generated `zsh/` entry names
before and after actual Zsh startup, and requires a completion dump to appear
in the user's cache rather than in the sealed generation before re-applying.
Completion source fixtures invoke Bash through the test environment's PATH,
with Bash explicitly declared as a Nix check-time input. They must not assume
a host `/bin/bash` or skip the checks when the interpreter is absent. Missing
interpreter diagnostics must name the required test dependency; production
Zsh startup and installed runtime dependencies are unchanged.
Generation search roots precede existing XDG_DATA_DIRS/XDG_CONFIG_DIRS, or their
standard defaults if absent. XDG_CONFIG_HOME remains unchanged, preserving the
precedence of a user's explicit qt6ct configuration. A nonempty explicitly
inherited QT_QPA_PLATFORMTHEME remains authoritative; otherwise use qt6ct.

Apply this environment to both fixed consumers and direct argv launches from
the session worker (including the existing browser dispatcher). Construct the
worker's environment once from the login selection at daemon startup; WM
restart still reloads that same selection. Do not mutate the daemon process
environment or import these overrides into shared systemd/D-Bus activation
state. Newly execed applications inherit it; reused processes and shared
activation services retain the documented boundary above.

Source tests must capture real child argv/environment for terminal, launcher
and worker launches, preserve caller environment and user files, and exercise
apply B followed by fresh children still selecting login A. CI reuses the real
Foot/Yazi/btop/zsh/Starship/GTK3/GTK4/Qt6 probes, extended to launcher descendants.
The stopping-child fixture SHALL observe the kernel's child-stop notification
after argv capture before inspecting files or sending continuation. Merely
creating or writing a PID file is not readiness: it can expose empty bytes,
precede complete argv capture, and race a continuation against the later stop.

## B1 — Waybar comparison, not automatic replacement

Compare the existing bar with packaged Waybar on the same CI-built River session.
Use upstream modules for commodity metrics and a narrow event-fed Realm adapter
for orbit/layout/mode/title/chord state. Do not assume River tag modules reflect
Realm's ledger. Avoid adding polling for state already available as events.
Sampled system metrics and a clock may use bounded periodic updates.

Preserve six orbit states, layout/mode, focused title, clock, CPU/memory/network/
battery, which-key and the grimoire. Preserve readable glyphs/fallbacks, scaling,
reserved workarea, fullscreen behavior and focus. A temporary hybrid experiment
is allowed; do not establish permanent duplicate bars or supervisors.

Record cold-start-to-usable, state-to-visible update latency, idle CPU and memory
with environment, versions and sample method. Compare against the existing
ARCHITECTURE.md targets; report misses rather than masking them. Renderer-specific
damage rectangles, pure-Rust code and an exact number of timers are not acceptance
criteria for the alternative. Select it only with functional evidence and a
recorded comparison; otherwise retain the existing bar. New custom metric/render
work waits for that result. Repairing a real current-bar usability defect does not.

The CI-only comparison keeps its three titled terminal shells interactive for
later title probes. Its transient Waybar units may already be collected when
cleanup runs; cleanup succeeds only when Waybar and its adapter are gone and a
real which-key toggle and focused-window change still work through the baseline.
Retain structured evidence and screenshots from a failed VM run as well as a
passing run. A full-frame OCR match that the existing bar or a terminal can
supply is only an observation of screenshot timing, not proof of Waybar's first
matching frame. Report the actual elapsed idle sample span and cadence, and
observe the ledger's fullscreen state before recording fullscreen screenshots.

## Verification obligations before implementation is declared complete

### Native consumer acceptance (Ubuntu/Fedora)

The installed graphical-session CI journey opens Foot through the real default
binding, observes its managed window and actual Zsh child, and records their
selected-generation environment and Foot arguments.
The process probe identifies exactly one Realm-launched terminal by its exact
selected-login `--config=` argument, not by the `foot` process name alone:
distribution-owned Foot servers may share that name. Zero or multiple matching
terminals fail, and terminal closure requires the selected terminal and its
shell to exit without requiring an unrelated distribution server to stop.
The native fixture records the installed Foot version and independently checks
the generated modern-then-legacy configuration variants with that binary's
`--check-config`; the launched arguments must match the supported selection.
Neither variant parsing successfully is a failure, not a fallback waiver.
After applying generation B, the login record and retained terminal/shell remain
on A. Keyboard input to that
shell must execute a fixture script and open the installed private Yazi on a
fixture directory; Yazi must inherit A's configuration, not B.
The fixture observes the actual Yazi process before reading the shell/tool proof
files written sequentially before Yazi executes; an earlier marker alone is not
a completion barrier. Record executable identities, shell execution evidence,
compositor state and complete framebuffers;
quit Yazi and the terminal and verify clean process/window closure. Missing
private Yazi/Starship is a failure, not a skipped acceptance. This CI slice
depends on SPEC 0024's native tool delivery (#236); it does not add distro tool
substitutes, enable live theme changes, or claim GTK/Qt, portal/browser or
next-login/relogin acceptance.

The separate installed Ubuntu 24.04 and Fedora 44 toolkit slice must close
that GTK/Qt claim with real distro-packaged GTK3 widget factory, GTK4 widget
factory and qt6ct clients. Install only the GTK example and tracing CI fixture
packages. The native Realm package dependency must already supply qt6ct and
its platform-theme plugin before any fixture installation, because the session
selects `QT_QPA_PLATFORMTHEME=qt6ct`; installing that plugin as a test fixture
would mask a broken user installation. Record each actual package owner and
version, and fail when a required executable or plugin is missing. After the
earlier native consumer slice has
prepared generation B while login A stays selected, start each toolkit through
the shipped launcher binding and a fixture desktop entry; do not launch it from
SSH or count a shared portal service as the selected client. For each client,
require its real managed focused window and a toolkit-specific visible control
label independent of the window title (`togglebutton` for GTK3, `Page 1` for
GTK4, and `Appearance` for qt6ct),
retain a complete framebuffer, prove a successful open of A's GTK3/GTK4 named
theme CSS or both A's qt6ct configuration and colour file, reject relevant
theme/configuration diagnostics, and require a clean exit after normal close.
Also record the launched process's selected environment, exact observed title,
stderr and relevant file-open trace on failure. A simple inherited GTK_THEME or
QT_QPA_PLATFORMTHEME value on Foot, Zsh or Yazi does not prove toolkit use.
The CI-only probe runs after browser OCR dependencies are present and before
normal relogin; diagnostic-only VM runs must retain their existing early exit.

The separate native relogin slice requests normal session quit and observes the
fixture's existing SDDM automatic relogin, without restarting the display manager
or manually removing login state. It requires the prior graphical logind session,
entry owner, River, WM and bar process identities to end, a distinct graphical
session and new live identities, and the prepared B selection in the new login
record. Open a real terminal through its binding and verify its Foot/Zsh children
consume B and form a managed window; close it normally. Retain before/after
identity, selection, state and framebuffer evidence. A timeout or stale A owner
fails rather than repairing the session in the fixture.

The NixOS fixture uses Ly, whose initial automatic login does not repeat after
logout. Its equivalent next-login probe gives only the test user a known fixture
password and selects Ly's password input field. After normal quit, observe the
visible greeter before submitting that password through real keyboard input.
Reuse the native identity/selection assertions and verify real Foot/Zsh consume
B. Do not restart the display manager, bypass PAM, remove stale runtime records,
or change production authentication policy to make this test pass.

### Overlap scope resolved

The owner confirmed one session on 2026-09-27 in response to the explicit
one-Realm-session-per-user recommendation. This preserves SPEC 0005's single
active same-UID claim. Same-user concurrent Realm sessions are not an MVP
requirement. The login handoff must survive window-manager restart without
reselecting the latest prepared theme; a second login must not overwrite the
active selection or its activation environment. Different users remain isolated.

| ID | Required evidence |
| --- | --- |
| V1 | Login with A; apply B; both an existing app and a newly launched app in that session still use A; next login uses B. |
| V2 | Failed render/publication does not disturb active A or replace the last complete next-login selection. |
| V3 | A competing same-user login is rejected without affecting the active selection; different-user sessions and a single-instance app demonstrate the documented scope; another desktop's files/environment are not overwritten. |
| V4 | Actual packaged foot, Yazi, btop, shell, GTK/Qt and launcher consume the session configuration; no placeholders counted as coverage. |
| V5 | Logout/restart leaves no Realm-owned orphan client; legacy theme data is not deleted while possibly in use. |
| V6 | Waybar comparison covers B1 behavior, measurements and inspected real screenshots; failures remain visible. |

Write failing tests for changed behavior before implementation. Packages,
installed-consumer tests and VM verification run in CI only. This spec update
does not make current code conformant or mark any of V1–V6 passed.

## Qt6 runtime delivery

The selected `QT_QPA_PLATFORMTHEME=qt6ct` requires its platform-theme plugin
in the shipped runtime, not a later test dependency. Debian/Ubuntu `Depends`
and Fedora `Requires` must include `qt6ct`. Official inventories confirm
[Ubuntu 24.04's executable and Qt6 plugin](https://packages.ubuntu.com/noble/amd64/qt6ct/filelist)
and [Fedora 44's executable and Qt6 plugin](https://packages.fedoraproject.org/pkgs/qt6ct/qt6ct/fedora-44-updates.html).
Native acceptance must check plugin ownership/presence before installing any
toolkit test applications; installing qt6ct in that step would conceal a
broken production dependency.

The NixOS module installs `qt6Packages.qt6ct` and provides its plugin root via
the Realm WM service's `QT_PLUGIN_PATH`, inherited by launched consumers.
Use the package's `qtbase.qtPluginPrefix`, not a guessed store layout; keep
normal Nix module override semantics. Do not select a global Qt platform theme
or expand the shared session-import allowlist. The pinned
[qt6ct recipe](https://github.com/NixOS/nixpkgs/blob/9fbb54b33e91ee4ca368e35a78e0613c720600b3/pkgs/tools/misc/qt6ct/default.nix)
installs into that prefix; the pinned
[Qt wrapper hook](https://github.com/NixOS/nixpkgs/blob/9fbb54b33e91ee4ca368e35a78e0613c720600b3/pkgs/development/libraries/qt-6/hooks/wrap-qt-apps-hook.sh)
prefixes `QT_PLUGIN_PATH` rather than replacing inherited entries. The VM must
not supply qt6ct only as a fixture package: require its real plugin file and
plugin-root inheritance in the live WM and terminal, while retaining actual
Qt configuration/colour-file opens and explicit platform-selector checks.
Package presence alone is not plugin-consumption evidence.
The supported Home Manager module must supply the same package and scoped
plugin-root environment in its own WM unit, preserving its existing PATH:
that user-level unit takes precedence over the NixOS-generated unit.

## Queue disposition

- Keep lock/idle (#79), portals (#69/#102), login/session (#68/#70), XWayland
  (#107), distro installation and week-long use (#78) on the MVP path.
- Rescope #117/#135 and theme asset issues #25–#31 to T1/T2. They no longer wait
  for SPEC 0012's transfer/gate machinery. PRs #240/#241 require reconciliation;
  retain useful assets and consumer probes, not obsolete lifecycle obligations.
- Keep #43/#45–#53 as functional bar outcomes, evaluated through B1. Shelve #54's
  renderer-specific damage/benchmark harness; B1 still measures performance.
- Shelve generation-aware live upgrade (#22). Retire automatic upstream-tool
  replacement (#88); shelve custom launcher/harness/dialog/compositor work
  (#82–#87, #89–#92) without treating it as an eventual mandatory replacement.
- Do not close historical implemented generation issues as regressions, reopen
  them, or delete their code merely because their contract is no longer required.
