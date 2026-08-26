# ADR 0002 — Borrow a compositor first; write ours later

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

The handoff asks for a custom Smithay compositor long-term and names `river` or
`niri` as the interim. The MVP cut line is "one person can use helm as their
only desktop for a week" (`docs/MVP.md`), and M3 is that MVP.

A Wayland compositor that is genuinely daily-drivable is a twelve-month job at
minimum: xdg-shell, layer-shell, output management, fractional scaling, DRM
leasing, XWayland, session locking, input methods, tablets, clipboard and
primary selection, screencopy. None of that is the product. The product is the
ledger, the theme pipeline, the bar and the key model, and all four are testable
without owning a compositor: `helm-core` is already roughly 2,900 lines of
source and tests with no Wayland dependency at all.

Building the compositor first means nobody, including us, uses helm for a year.

## Decision

1. `helm-session` talks to a `WmBackend` trait. It never calls a compositor
   directly.
2. `NiriBackend` implements that trait against niri's IPC socket for M2–M4.
   niri is Rust, built on Smithay, and already daily-drivable.
3. `NativeBackend` implements the same trait in-process against
   `helm-compositor` from M5.
4. Everything else — clients, themes, keymap, ledger — is written against the
   trait. No file outside `crates/helm-session/src/backend/niri.rs` may mention
   niri.
5. niri stays a *stopgap behind a seam*, per standing order S8.

## niri concept mapping

niri's window model is a scrollable strip of columns per workspace. helm's model
is an ordered list per orbit with a projected layout. **These are not the same
model.** `NiriBackend` is a projection with known lossy edges, not a passthrough.

| helm concept | niri concept | Fidelity |
|---|---|---|
| Orbit (6, fixed, runes ᚠᚢᚦᚨᚱᚲ) | Workspace | Good, with a caveat: niri workspaces are dynamic and it creates/removes them as they empty. helm pins six named workspaces at startup and refuses to let the count drift |
| Ledger order (`Vec<WinId>`, index 0 is master) | Left-to-right column order in the scrollable strip | Good for order; the strip has no notion of a master slot, so index 0 is master only by our convention |
| `focus_step(Next/Prev)` | `focus-column-right` / `focus-column-left` | Good, except wrap-around: helm wraps at the ends, niri stops. Backend must detect the end and jump |
| `swap(Next/Prev)` | `move-column-right` / `move-column-left` | Good |
| Triptych (640px master + 2-column stack) | No equivalent | **Lossy.** niri has column widths but no nested row split inside a column with independent per-cell heights. The closest approximation is a wide first column plus columns of stacked windows, which is not the reference desktop |
| Mono | A single full-width column, or niri's own fullscreen | Approximate. helm's mono keeps the bar's reserved strip; niri fullscreen does not |
| Even (equal grid) | No equivalent | **Lossy.** Same reason as triptych |
| Stow | No equivalent | **Lossy.** The nearest behaviour is moving the window to a scratch workspace and back. Window identity survives; niri's own focus history does not |
| Fullscreen (`mod+f`, covers the bar) | `fullscreen-window` | Good |
| Workarea (output minus 32px bar and 26px which-key) | layer-shell exclusive zones | Good; niri honours exclusive zones |
| Undo (restore an older ledger) | No equivalent | **Lossy by construction.** Undo replays as a sequence of niri actions to reach the target order. The end state matches; the intermediate frames do not, and niri's animations make the replay visible |

The honest summary: on niri, helm gets orbits, ordering, focus, swap, fullscreen
and the bar. It does not get the reference triptych geometry. M2's bar and
keymap are fully faithful; the *tiling* is an approximation until M5.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **river** | Its layout protocol is the best structural fit in the field: river delegates tiling to an *external layout generator* over `river-layout-v3`, which is very close to what `layout::project` already is. We could ship the real triptych geometry on someone else's compositor, which niri cannot give us. River is also stable and well-regarded | Zig, not Rust, so it shares no toolchain, no CI story and no eventual code path with `helm-compositor`. It is pre-1.0 (*assumption at time of writing; re-check before acting on this row*). Its layout protocol delivers *dimensions only*, so stow and orbit-pinning still need side channels. This was a genuinely close call and the loss is real: we are trading exact geometry for a Rust codebase we can eventually absorb |
| **Smithay from day one** | It is where we end up anyway; no throwaway backend, no lossy mapping, the ledger drives real pixels from the first commit | Twelve months before anyone logs in. Violates the MVP cut line outright, and would mean designing the theme pipeline and bar against a compositor that does not exist yet |
| **wlroots with C bindings** | The most mature compositor library by a wide margin; tinywl is a working starting point | Drags a C build and `unsafe` FFI into a workspace that sets `#![forbid(unsafe_code)]`. Also a full compositor build, so it costs almost as much as Smithay without the payoff of being the destination |

## Consequences

### Good

- Someone can log into helm at M3 rather than M5.
- niri is Rust on Smithay, so debugging it teaches us the library we will use.
- The `WmBackend` seam is forced into existence early, when it is cheap. A trait
  designed with two implementations in mind is a better trait.

### Bad

- The reference triptych geometry from `Desktop v3.dc.html` is not achievable on
  niri. Early users see an approximation and may reasonably judge helm on it.
- Two window models running at once means state divergence is possible: if niri
  moves a window we did not ask it to, the ledger is wrong until we reconcile.
  The backend must subscribe to niri's event stream and treat it as authority
  for existence, while helm remains authority for order.
- niri animates. helm's whole aesthetic is that nothing moves (ADR 0009). We
  must ship a niri config that disables animations and hope it stays possible.
- We inherit niri's release cadence and its config format's stability.

### Neutral

- A second backend is not wasted work: `NiriBackend` doubles as the integration
  test harness for `WmBackend` once `NativeBackend` exists.
- niri's IPC surface and action names are version-sensitive. The mapping table
  above is written against the niri release pinned in `packaging/`; treat it as
  an assumption to re-verify on every bump.

## Reversal

Low, by design. Swapping the backend touches `crates/helm-session/src/backend/`
and the packaging dependency list. Nothing in `helm-core`, `helm-bar`,
`helm-theme` or `helm-ctl` changes. Estimated cost of adding a third backend
(river, say) once the trait is settled: a few days.

Signals to reconsider: niri's IPC breaking across releases faster than we can
track it; losing the ability to disable niri's animations; or user reports that
the triptych approximation is why they stopped using helm, which would argue for
pulling `helm-compositor` forward.

## Guard

- *Planned (M2):* `helm-session` integration test that drives `NiriBackend`
  against a headless niri and asserts that a scripted sequence of ledger
  mutations produces the matching niri window order.
- *Planned (M2):* a compile-time check — a CI grep asserting that the string
  `niri` appears nowhere in the workspace outside
  `crates/helm-session/src/backend/`, `packaging/` and `docs/`. This is the
  guard for the seam itself, which is the part of this decision most likely to
  erode quietly.
- `layout::tests::triptych_matches_the_reference_desktop` stays the definition
  of correct geometry regardless of backend, so the gap above is measurable
  rather than merely asserted.

## Needs a human

**The lossy rows in the mapping table need a decision, not an implementation.**
Three options, and they are not mutually exclusive:

1. **Ship the approximation and say so.** `NiriBackend` maps triptych onto the
   nearest scrollable-strip arrangement, and the release notes state plainly
   that exact tiling arrives with `helm-compositor`. Cheapest; risks first
   impressions.
2. **Restrict the niri-backed layouts.** Offer only `mono` and a simple even
   split on niri, and expose triptych only on `NativeBackend`. Honest, but it
   makes the two backends visibly different desktops.
3. **Reopen river.** river's external layout generator could run
   `layout::project` directly and give exact geometry today, at the cost of a
   Zig dependency and a backend with no future.

**Recommendation: option 1 for M2, with option 2 as the fallback if the
approximation reads as broken rather than as different.** Option 3 stays on the
table only if `helm-compositor` slips past M5.

The second open question: **does stow map to a hidden workspace, or does
`NiriBackend` refuse `mod+s` entirely on niri?** Refusing is more honest;
hidden-workspace is more useful. Recommendation: hidden workspace, with
`helm ctl doctor` reporting it as an approximation.

Both are tracked as `needs-human` issues (standing order S3).
