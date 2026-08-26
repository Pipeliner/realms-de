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
    pub exact_geometry: bool,     // can we place windows at arbitrary rects?
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
| Placement rectangle | `river_node_v1::set_position` + `river_window_v1::propose_dimensions` | Exact |
| Ledger order | `river_node_v1::place_above` / `place_below` / `place_top` / `place_bottom` | Exact |
| Stow | `river_window_v1::hide` / `show` | Exact |
| Focus | `river_seat_v1::focus_window` / `clear_focus` | Exact |
| 1px seams | `set_borders`, drawn by the compositor | Exact |
| Fullscreen | `fullscreen` / `exit_fullscreen` | Exact |
| Atomic relayout | the `manage` → `render` sequence | Exact, and frame-perfect by construction |

Two consequences worth stating plainly, because they cut both ways:

1. **`apply()` maps onto one `manage` sequence.** river applies window-management
   state atomically between `manage_start` and `manage_finish`, which is exactly
   the guarantee the projection wants: a relayout is never observed half-done.
2. **If `helm-session` dies, river has no window management at all.** Under niri
   a crashed session daemon left a working, if unmanaged, desktop. Under river it
   leaves windows unplaced. The session daemon therefore needs a restart policy
   sharper than "on-failure and hope", and that is a requirement on M2, not a
   nicety. See ADR 0013.

The protocol is registry-classified *unstable*, against which river's maintainer
pledges "we do not break window managers". Both are true. helm pins a tested
river version and treats a protocol bump as a tracked event.

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

1. **No timers except the clock.** Every other module is push-driven. A module
   that can only be polled must justify itself in its PR.
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
