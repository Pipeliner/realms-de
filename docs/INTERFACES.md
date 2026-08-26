# Interface contracts

> **Status: provisional.** These are the seams named in
> [ARCHITECTURE.md](ARCHITECTURE.md). They are written down *before* the crates
> that implement them so that M1 and M2 can be built in parallel without two
> components inventing the same type twice.
>
> Signatures here are a design commitment, not final code. Changing one means
> updating this file in the same commit.

---

## 1. `WmBackend` — the compositor seam (ADR 0002, 0003)

The whole point of this trait is that `helm-session` never learns which
compositor it is talking to. `RiverBackend` implements it in phase 1 by *being*
river's window manager; `NativeBackend` implements it in-process against
`helm-compositor` in M5. Nothing above this line changes when we swap them.

```rust
/// A window manager helm can drive.
///
/// Implementations translate helm's ledger operations into whatever the
/// underlying compositor understands, and translate the compositor's events
/// back into ledger deltas. They own no policy: the ledger decides what should
/// happen, the backend only makes it so.
pub trait WmBackend: Send {
    /// Human-readable name, shown by `helm ctl doctor`.
    fn name(&self) -> &str;

    /// Connect, and report what the backend can actually honour.
    fn connect(&mut self) -> Result<Capabilities>;

    /// Apply a projection. Called only when the projection has changed.
    ///
    /// Implementations must be idempotent: submitting the same placements
    /// twice must not produce a visible change or a second frame.
    fn apply(&mut self, placements: &[Placement]) -> Result<()>;

    /// Give a window keyboard focus.
    fn focus(&mut self, win: WinId) -> Result<()>;

    /// Ask a window to close politely; the compositor may refuse.
    fn close(&mut self, win: WinId) -> Result<()>;

    /// The workarea currently available for tiling.
    fn workarea(&self) -> Workarea;

    /// Block until the next backend event, or until `deadline`.
    fn next_event(&mut self, deadline: Option<Instant>) -> Result<Option<BackendEvent>>;
}

/// What a backend can and cannot do, so helm degrades honestly rather than
/// pretending. `helm ctl doctor` prints this.
pub struct Capabilities {
    /// True when the *rendered* rectangle is exactly the projected one.
    ///
    /// Not "can we ask for arbitrary rects" — we always can. This asks whether
    /// what lands on screen matches, which `propose_dimensions` alone cannot
    /// promise because clients may quantise. True at `river_window_v1` >= 3,
    /// where `set_content_clip_box` lets helm clip to the exact tile; false
    /// below it, with `"unclipped-dimension-quantisation"` in `unsupported`.
    pub exact_geometry: bool,
    pub server_side_borders: bool,
    pub hide_show: bool,          // is stow expressible?
    pub explicit_ordering: bool,  // can we set stacking order directly?
    pub fullscreen: bool,
    pub unsupported: Vec<&'static str>, // named helm behaviours this backend cannot honour
}

/// Something the compositor did that the ledger needs to know about.
pub enum BackendEvent {
    WindowOpened { win: WinId, app_id: String, title: String },
    WindowClosed(WinId),
    TitleChanged { win: WinId, title: String },
    FocusChanged(Option<WinId>),
    WorkareaChanged(Workarea),
    /// The compositor moved a window itself. helm treats this as advisory: the
    /// ledger remains the truth and the next projection will overrule it.
    GeometryDrifted { win: WinId, rect: Rect },
    Disconnected,
}
```

### Why river fits: helm *is* the window manager

river 0.4 removed window-management policy from the compositor entirely and
defers it to an external process over `river-window-management-v1`. helm is that
process. The protocol's vocabulary is close enough to the ledger's that the
backend is a translation rather than an approximation:

| helm concept | river request | Fidelity |
|---|---|---|
| Placement rectangle | `river_node_v1::set_position` + `river_window_v1::propose_dimensions` | **Approximate** — see the quantisation note below |
| Ledger order | *helm's own*, expressed through the positions it computes | Exact, because helm owns it outright |
| Stacking (mono occlusion, overlays) | `river_node_v1::place_top` / `place_bottom` / `place_above` / `place_below` | Exact |
| Stow | `river_window_v1::hide` / `show` — *rendering* state, so the window stays managed and stays in the ledger | Exact, and a closer match to `Orbit::stowed` than we expected |
| Focus | `river_seat_v1::focus_window` / `clear_focus` | Exact. Note `focus_exclusive` / `focus_non_exclusive` / `focus_none` are **events**, not requests: helm is told about exclusive focus, it does not grant it |
| 1px seams | `set_borders`, drawn by the compositor | Exact |
| Fullscreen | `fullscreen` / `exit_fullscreen` | Exact. Whether the bar draws over a fullscreen window is decided by the bar's chosen *layer*, not by node ordering: `river-layer-shell-v1` exposes no node and no ordering request at all |
| Window identity | `river_window_v1` `identifier` (up to 32 printable ASCII bytes) | **Requires a mapping.** `WinId` is a `u64`, so helm-session holds a bijection and allocates ids from a monotonic counter keyed on river's identifier. The never-reused property survives, but via helm's counter rather than river's string |
| Workarea | `river_layer_shell_output_v1::non_exclusive_area` | Exact — arrives as an event. It is a free rectangle in global coordinates, *not* the `Workarea::new(w, h, top, bottom)` shape, so the backend converts |
| Atomic relayout | the `manage` **and** `render` sequences, in that order | Exact, but it is **two** phases and helm must respect the boundary — see below |

**A placement spans both phases.** `propose_dimensions` is window-management
state; `set_position` is *rendering* state. So a single `apply()` is not one
manage sequence: sizes go between `manage_start` and `manage_finish`, then
positions go after `render_start`. Submitting a position in the management phase
raises `error::sequence_order` and kills the connection — this is a protocol
error, not a style preference, and it is the single easiest way to get river
integration wrong.

`place_*` orders the **render list**, not the ledger. The ledger is *layout*
order, which helm computes itself and expresses as positions; the `place_*`
requests exist for mono's occlusion stack and for overlay surfaces. Faithful
either way, but not for the reason a first reading suggests.

**The one genuinely approximate row.** `propose_dimensions` is a *proposal*: the
protocol explicitly anticipates clients quantising it, terminals to their cell
size being the named case. A terminal that rounds 700×580 down to 696×576 puts a
4×4 hole in a layout whose entire premise is exact tiling, and
`every_layout_tiles_exactly_for_every_plausible_size` would still pass while the
screen showed cracks — the test checks the projection, not what the client did
with it. river offers `set_content_clip_box`, which clips content to a rect and
draws borders around the intersection, so helm can propose at or above the tile
and clip to the exact rectangle. That is the plan; it is an M2 experiment with
its own guard, not a solved problem.

### What helm must implement, not merely call

Under river, a window manager is not only a client of the WM protocol. river's
`protocol/` directory holds **six** protocols, and five of them are obligations
helm must serve. The first is load-bearing and the last two are the difference
between a desktop and a demo on a laptop:

| Protocol | What helm owes it | Consequence if unimplemented |
|---|---|---|
| `river-layer-shell-v1` | Serve layer-shell on river's behalf | **The bar does not appear at all.** `wlr-layer-shell` works under river only if the window manager implements it |
| `river-xkb-bindings-v1` | The entire keymap, **and key repeat for bound keys** | No keybinding works. `ensure_next_key_eaten` and `ate_unbound_key` (on `river_xkb_bindings_seat_v1`, reached via `get_seat`) are purpose-built for chorded submaps, which is exactly helm's chord model. `stop_repeat` establishes that repeat for bound keys is the window manager's job, so helm owns a second timer — armed only between `pressed` and `released`, which is the justification ADR 0009's no-timers rule requires |
| `river-input-management-v1` | Seats, repeat rate, pointer config | No input configuration |
| `river-xkb-config-v1` | Keymap selection (`set_layout_by_name`), the `layout` event, caps and num lock | Layouts are frozen at whatever `XKB_DEFAULT_LAYOUT` was when river started, with no way to switch |
| `river-libinput-config-v1` | Tap-to-click, drag, natural scroll, accel profile and speed, click and scroll method, calibration | **A laptop has no tap-to-click and no way to get one.** Under river 0.4 there is no input config file — the window manager *is* the input configuration |

This is a materially larger phase-1 surface than "write a backend", and M2 is
scoped accordingly.

Two consequences worth stating plainly, because they cut both ways:

1. **`apply()` maps onto one `manage` sequence.** river applies window-management
   state atomically between `manage_start` and `manage_finish`, which is exactly
   the guarantee the projection wants: a relayout is never observed half-done.
2. **`helm-session` is now on the compositor's input path, with a hard liveness
   requirement.** Under niri, a crashed session daemon left a working if
   unmanaged desktop. Under river it leaves windows unplaced and keys dead, and
   the protocol has an `unresponsive` error: `modifiers_update` warns that the
   compositor's input buffering is finite. **A stall is a session failure, not a
   slow frame.** Nothing in `helm-session` may block — not a theme apply, not a
   socket write to a wedged subscriber. This promotes the frame budgets in
   ARCHITECTURE §4 from performance goals to correctness requirements. See
   ADR 0013.

On stability: `river-window-management-v1` is **declared stable** as of river
0.4.0, with a forward-compatibility pledge to 1.0.0 — no `z` prefix, no
`unstable/` directory, interfaces already at v5. (An earlier draft of this
document called it registry-classified unstable. That was wrong: the
work-in-progress language came from a tracking issue that predates the release.)
The residual risk is not a protocol classification but trust in a single
maintainer of a pre-1.0 project, which is a different and smaller thing. helm
pins a tested river and treats a protocol bump as a tracked event.

ADR 0002 records the superseded plan to ship on niri, and the mapping table that
argued us out of it — worth reading before anyone proposes going back.

---

## 2. Template contract — `helm-theme` (ADR 0005)

One palette in, many themed files out, atomically.

```rust
/// A file helm generates from the palette.
pub struct Template {
    /// Stable id, e.g. "gtk4", "foot", "yazi".
    pub id: &'static str,
    /// Source text with `{{ path.to.value }}` placeholders.
    pub source: &'static str,
    /// Where the rendered file lands, relative to $XDG_CONFIG_HOME.
    pub target: PathBuf,
    /// How live consumers are told to re-read it.
    pub reload: Reload,
}

/// How a themed program is told the theme changed.
pub enum Reload {
    /// Nothing to do: read at next start.
    None,
    /// Send a signal to every process matching this name.
    Signal { process: &'static str, signal: i32 },
    /// Run a command, e.g. `gsettings set ...`.
    Command(Vec<String>),
    /// Notify helm's own clients over the control socket.
    HelmClients,
}

/// Render every template and swap them in as one step.
///
/// Contract: each file is written to `<target>.helm-tmp` and `rename(2)`d into
/// place, then reloads are fanned out once, after every file is in place. A
/// half-applied theme — new terminal colours against an old GTK stylesheet —
/// must never be observable, even if the process is killed mid-apply.
pub fn apply(palette: &Palette, root: &Path) -> Result<Applied>;

pub struct Applied {
    pub written: Vec<PathBuf>,
    pub unchanged: Vec<PathBuf>,   // byte-identical; not rewritten, not reloaded
    pub reloaded: Vec<&'static str>,
    pub elapsed: Duration,         // budget: < 150 ms (ARCHITECTURE.md §4)
}
```

**Placeholder vocabulary.** Templates address the *derived* palette, so
`contrast` is already folded in and no template ever applies it itself:

| Form | Example | Yields |
|---|---|---|
| `{{ accent.violet }}` | | `#a692ec` |
| `{{ accent.violet.bare }}` | | `a692ec` |
| `{{ accent.violet.rgba(0.3) }}` | | `rgba(166, 146, 236, 0.3)` |
| `{{ accent.violet.over(background.pane, 0.3) }}` | | flattened hex, for formats without alpha |
| `{{ metrics.bar_height }}` | | `32` |
| `{{ typography.family }}` | | `IBM Plex Mono` |

An unknown placeholder is a hard error at render time, not an empty string. A
silently blank colour is exactly the bug this whole design exists to prevent.

---

## 3. Bar render contract — `helm-bar` (ADR 0008, 0009)

The bar is a pure function of `HelmState` plus the palette. It owns no state
beyond its Wayland surface.

```rust
/// Draw one frame. Called only when the state or the palette changed.
fn render(state: &HelmState, palette: &Palette, probe: &Probe, canvas: &mut Pixmap) -> Damage;

/// The region that actually changed, so the compositor is handed a damage
/// rectangle rather than a whole-surface repaint.
pub struct Damage(Option<Rect>);
```

Rules, enforced by review and by the budgets in ARCHITECTURE.md §4:

1. **The bar owns no timer at all.** Every value it draws arrives in
   `HelmState`. Four of the mockup's modules — cpu, mem, gpu temperature and the
   `↑ 18k ↓ 1.2M` throughput half of net — are *rates over counters*, and the
   kernel exposes no event for those; no bar on any platform gets them without
   sampling. So the sampling lives in **one shared sampler in `helm-session`**,
   off the window-management event loop, and is the single documented exception
   to ADR 0009's no-timers rule. The bar stays a pure function of state, which
   is the property that actually mattered.
   The clock ticks to the next **minute** boundary, not every second: the design
   shows `14:32`, so 59 of every 60 wakeups would redraw nothing.
2. **No redraw when nothing changed.** `HelmState::renders_same_as` gates the
   frame before any drawing happens.
3. **Every glyph goes through `Probe::resolve`.** Drawing a raw `char` from the
   inventory bypasses the fallback contract and is how tofu ships.
4. **Damage, not repaint.** A clock tick must damage the clock, not the bar.

---

## 4. Control socket — client side (ADR 0004)

```rust
/// A connection to helm-session.
impl Client {
    /// Connect and complete the version handshake. Refuses on mismatch rather
    /// than guessing at field meanings.
    pub fn connect() -> Result<Client>;
    pub fn request(&mut self, req: Request) -> Result<Response>;
    /// Subscribe; yields a state snapshot on every change until dropped.
    pub fn subscribe(self) -> Result<impl Iterator<Item = Result<Event>>>;
}
```

The wire types (`Request`, `Response`, `Event`, `HelmState`) already exist in
`helm-core::ipc` and `helm-core::state` and are the normative definition; this
is only the ergonomic wrapper.

---

## 5. What is deliberately *not* an interface

- **The ledger.** There is one implementation and there will only ever be one.
  Making it a trait would invite a second source of truth, which is the exact
  failure ADR 0001 exists to prevent.
- **The layout projection.** Same reason: layouts are an enum with a pure
  function, not a plugin surface. A layout that cannot be expressed as
  `fn(&Ledger, Workarea) -> Vec<Placement>` is a layout helm does not want.
