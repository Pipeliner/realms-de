# Reuse-first MVP reassessment

Date: 2026-09-27. Status: **research and recommendation, not an accepted ADR**.
Requested by the owner: radically reconsider existing programs and reusable code
instead of building the desktop from scratch. Assessed against main `b3d8cd9`,
MVP.md, ARCHITECTURE.md, and ADRs 0008, 0013, 0017 and 0018.
No runtime replacement, package installation, or contract change is authorized
by this record. Upstream capabilities below were checked in primary sources;
integration compatibility and performance have not been demonstrated here.

## Conclusion

Realm should own its distinctive window policy, not a general desktop stack.
Keep the ledger/projection/orbits/undo/chord semantics and River adapter. Reuse
complete programs for commodity functions, with configuration or a narrow
adapter before considering a fork. Reconsider expensive implementation-specific
contracts, not just the libraries used to implement them.

Already working custom code need not be rewritten merely to improve a diagram.
Compare remaining integration and maintenance work, not sunk development cost
or lines of code alone. Do not create permanent dual implementations.

## Ranked decisions

| Area | Recommendation | What remains Realm-specific / adoption condition |
| --- | --- | --- |
| Lock and idle | Finish integration of packaged swayidle + swaylock; no custom locker or lid listener | Preserve approved host lid policy, lock before suspend, dim at 300s, lock/blank at 600s. Verify River protocols, PAM and actual suspend ordering in CI/hardware. |
| Portals | Use xdg-desktop-portal-wlr plus GTK backend, not a Realm portal implementation | wlr supplies ScreenCast/Screenshot, not FileChooser/Settings. Keep explicit routing and verify actual browser/application operations. |
| Bar and system indicators | First replacement experiment: Waybar + CSS + one Realm state-stream adapter | Native CPU/memory/network/battery/clock modules; custom modules for orbits/layout/mode/title. Preserve chord discovery, glyphs, focus, workarea and fullscreen behavior. Do not assume its River modules understand Realm or River 0.4. |
| Session lifecycle | Evaluate UWSM as the sole outer session manager, not an additional supervisor | It supplies systemd units, environment setup/cleanup, autostart and app slices. Integrate readiness and shutdown once; no overlapping ownership with existing units. |
| Theme generation | Reuse Tinted Theming templates and/or Stylix target knowledge; consider simpler session-scoped theme selection | Mapping Realm palette roles to Base16/Base24 is not automatic. Stylix is Nix-oriented, not a cross-distro launch-lifecycle solution. Existing generation contracts require an explicit decision before simplification. |
| Terminal, launcher, files, monitoring, prompt | Continue foot/fuzzel/Yazi/btop/zsh/Starship; ship configs, not replacements | Already chosen correctly. Recheck minimum required versions before adding private packages; retain private builds only where target distro packages cannot meet a demonstrated requirement. |
| Future custom launcher/overlay | Keep existing launcher for MVP; do not build a text-entry widget stack | A fuzzel menu is not automatically a passive which-key strip. Preserve working chord hints until a replacement proves those semantics. |
| Compositor | Keep River for the current product contract; remove any assumption a custom compositor is an inevitable destination | River already supplies the lower-level compositor. A custom compositor needs a concrete unmet requirement, not a roadmap milestone alone. |

## Architecture assumptions to challenge

**ADR 0008 rules out GTK on predicted startup/dependency costs.** Its custom
renderer also brings layout, hit testing, text shaping, buffer management and
event handling into our ownership. Waybar has distribution packages and existing
system modules. Compare cold start, idle CPU, memory, glyph rendering and the
actual design on the same CI VM before declaring GTK too expensive. Its custom
module supports a continuous JSON producer; a polling adapter is unnecessary.
Some system metrics still sample periodically. Do not promise zero timers.

**ADRs 0017/0018 make a theme into a durable process-ownership system.** Seals,
leases, launch gates and descendant-drain proofs enforce real accepted promises,
but those promises are stronger than simply shipping a coherent desktop theme.
UWSM cannot be called a drop-in replacement for them. The highest-leverage scope
proposal is: one theme selection per login session, standard application launch
semantics, and theme changes applied next login. This trades away per-launch
generation selection and its lifecycle guarantees. Keep existing safe cleanup
until the replacement is proven; do not simply delete leases or validation.
This proposal needs explicit acceptance before implementation.

**Rust-first need not mean every desktop process is Rust.** External programs
already form the desktop. A configured C++/GTK bar can be a smaller maintenance
commitment than Rust bindings plus a custom renderer. Conversely, introducing a
new framework without deleting responsibilities is not simplification.

## The radical alternative: a configured existing desktop

[Sway](https://swaywm.org/) has an existing distribution integration in the
[Fedora Sway Spin](https://fedoraproject.org/spins/sway/). This is stronger
deployment evidence than assuming a newly assembled stack is battle-tested.
It is a credible alternative if the product becomes a branded keyboard-first
desktop rather than Realm's exact ledger-driven window model.

It is **not** a verified drop-in implementation of exact triptych, stow and atomic
undo. ADR 0013 records the owner's explicit fidelity choice and rejection of
lossy niri semantics and dual backends. This reassessment does not reverse that
choice. A Sway fork could modify policy, but then Realm owns compositor patches,
rebases and regression risk; that is not clearly cheaper than the existing
external River WM. Prefer the River extension boundary for distinctive policy.

River links an example WM, tinyrwm, and a compatible-WM catalogue. Those are
research leads, not evidence of a mature replacement: their linked pages could
not be retrieved in this investigation. Do not label an example battle-tested.
Borrowing algorithms or implementation patterns requires protocol-version,
licence and behavior checks; replacing our working WM with another young WM
would not by itself reduce risk.

## Smallest useful next steps

1. Finish upstream lock/idle and portal integration: directly needed for daily use.
2. Run one CI-built Waybar experiment against the existing River session. Show
   the same orbit/title/state information and real screenshots; test glyphs,
   exclusive workarea, fullscreen and chord hints. Retain the current bar unless
   the experiment demonstrates a net reduction in remaining work. Amend ADR
   0008 before shipping a replacement.
3. Decide whether per-launch immutable theme generations are truly MVP product
   requirements. If not, supersede their contracts first, then evaluate UWSM
   against the simplified ownership model. Do not build a bridge preserving all
   the old machinery merely to claim adoption.
4. Audit private dependency packaging against actual used features and supported
   distro versions. Keep the currently accepted distro scope; don't quietly
   drop Ubuntu/Fedora to make a Nix-only tool look universal.

All package builds and installed-session verification remain CI-only. No ETA or
percentage reduction is established by this research. Configuration reuse is
preferred, small adapters second, maintained upstream extensions third, forks
last. Imported source needs a pinned origin, compatible licence/attribution and
tests of the behavior we rely on. This is normal adoption diligence, not a new
postponed-security-review MVP gate.

## Primary sources checked

- [Waybar](https://github.com/Alexays/Waybar): packaged bar, hardware/system
  modules, GTK dependencies. [Custom module](https://github.com/Alexays/Waybar/wiki/Module:-Custom):
  streaming JSON output, CSS classes and signal-triggered updates.
- [UWSM](https://github.com/Vladimir-csp/uwsm): systemd session management,
  environment handling, application slices and login/session lifetime binding.
  Its README calls the project stable but warns of breaking changes. This does
  not establish compatibility with Realm's generation contracts.
- [swayidle manual](https://github.com/swaywm/swayidle/blob/master/swayidle.1.scd):
  timeout/resume, logind before-sleep and waiting for swaylock readiness. The
  inhibitor is bounded by logind's InhibitDelayMaxSec; test failure timing.
- [xdg-desktop-portal-wlr](https://github.com/emersion/xdg-desktop-portal-wlr):
  supported portals and delegation to other backends.
- [Stylix](https://github.com/nix-community/stylix),
  [configuration](https://nix-community.github.io/stylix/configuration.html),
  [Tinty](https://github.com/tinted-theming/tinty): existing theming systems;
  candidate template/configuration reuse, not proof of Realm palette fidelity.
- [River](https://github.com/riverwm/river): separation of compositor and WM,
  stable protocol, example/catalogue links. Source is GPL-3.0-only; protocols
  MIT. Upstream prohibits LLM-generated contributions, including issue comments;
  do not plan automated upstream patches or reports as the maintenance strategy.

Maturity is comparative engineering judgment, not certification. Current
upstream main documentation is not proof a feature exists in our pinned or
distro-provided versions; adoption must verify the exact packaged versions.
