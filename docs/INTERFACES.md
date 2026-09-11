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

The whole point of this trait is that `realm-session` never learns which
compositor it is talking to. `RiverBackend` implements it in phase 1 by *being*
river's window manager; `NativeBackend` implements it in-process against
`realm-compositor` in M5. Nothing above this line changes when we swap them.

```rust
pub type BackendResult<T> = std::result::Result<T, BackendError>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendWindowId(pub String);

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("backend cannot honour capability {capability}")]
    Unsupported { capability: String },
    #[error("backend disconnected")]
    Disconnected,
    #[error("backend unavailable: {message}")]
    Unavailable { message: String },
    #[error("backend I/O failed: {message}")]
    Io { message: String },
}

pub enum SessionActionError {
    /// Recovery is incomplete or authoritative backend repair is pending.
    NotReady,
    /// The requested operation reached the compositor backend and failed.
    Backend(BackendError),
}

/// A window manager realm can drive.
///
/// Implementations translate realm's ledger operations into whatever the
/// underlying compositor understands, and translate the compositor's events
/// back into ledger deltas. They own no policy: the ledger decides what should
/// happen, the backend only makes it so.
pub trait WmBackend: Send {
    /// Human-readable name, shown by `realmctl doctor`.
    fn name(&self) -> &str;

    /// Connect, and report what the backend can actually honour.
    fn connect(&mut self) -> BackendResult<Capabilities>;

    /// Bind a compositor-stable window identity to Realm's allocated id.
    ///
    /// This operation is idempotent and error-atomic. Repeating the same pair
    /// after success is a no-op; an error leaves no binding installed, so the
    /// session can retry it before reading another backend event. A conflicting
    /// pair is an error. The binding must succeed before the window is included
    /// in another backend operation.
    fn assign_window(
        &mut self,
        backend_id: &BackendWindowId,
        win: WinId,
    ) -> BackendResult<()>;

    /// Apply a complete projection when it changed or a prior apply failed.
    ///
    /// While no error intervenes, implementations must be idempotent:
    /// submitting the same placements twice does not produce a visible change
    /// or a second frame. Before returning an error, an implementation
    /// invalidates every projection, diff, per-window and request cache. The
    /// next call must issue the complete requested projection even when it
    /// equals the last successful projection. Only success restores cache
    /// validity.
    fn apply(&mut self, placements: &[Placement]) -> BackendResult<()>;

    /// Give a window keyboard focus.
    fn focus(&mut self, win: WinId) -> BackendResult<()>;

    /// Ask a window to close politely; the compositor may refuse.
    fn close(&mut self, win: WinId) -> BackendResult<()>;

    /// The workarea currently available for tiling.
    fn workarea(&self) -> Workarea;

    /// The backend's readable descriptor for the session event loop's poll set.
    fn event_fd(&self) -> std::os::fd::RawFd;

    /// Block until the next backend event, or until `deadline`.
    fn next_event(&mut self, deadline: Option<Instant>) -> BackendResult<Option<BackendEvent>>;
}

/// What a backend can and cannot do, so realm degrades honestly rather than
/// pretending. This serialisable wire type lives in `realm_core::ipc`, and
/// `realmctl doctor` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// True when the *rendered* rectangle is exactly the projected one.
    ///
    /// Not "can we ask for arbitrary rects" — we always can. This asks whether
    /// what lands on screen matches, which `propose_dimensions` alone cannot
    /// promise because clients may quantise. True at `river_window_v1` >= 3,
    /// where `set_content_clip_box` lets realm clip to the exact tile; false
    /// below it, with `"unclipped-dimension-quantisation"` in `unsupported`.
    pub exact_geometry: bool,
    pub server_side_borders: bool,
    pub hide_show: bool,          // is stow expressible?
    pub explicit_ordering: bool,  // can we set stacking order directly?
    pub fullscreen: bool,
    pub unsupported: Vec<String>, // named realm behaviours this backend cannot honour
}

/// Something the compositor did that the ledger needs to know about.
pub enum BackendEvent {
    /// The connected backend has reported every window present at startup.
    InitialReplayComplete,
    WindowOpened { backend_id: BackendWindowId, app_id: String, title: String },
    WindowClosed(WinId),
    TitleChanged { win: WinId, title: String },
    /// Effective keyboard focus among managed windows changed.
    FocusChanged(Option<WinId>),
    /// A layer surface acquired or released exclusive keyboard focus.
    ExclusiveFocusChanged(bool),
    WorkareaChanged(Workarea),
    /// The compositor moved a window itself. realm treats this as advisory: the
    /// ledger remains the truth and the next projection will overrule it.
    GeometryDrifted { win: WinId, rect: Rect },
    Disconnected,
}
```

### Why river fits: realm *is* the window manager

river 0.4 removed window-management policy from the compositor entirely and
defers it to an external process over `river-window-management-v1`. realm is that
process. The protocol's vocabulary is close enough to the ledger's that the
backend is a translation rather than an approximation:

| realm concept | river request | Fidelity |
|---|---|---|
| Placement rectangle | `river_node_v1::set_position` + `river_window_v1::propose_dimensions` | **Approximate** — see the quantisation note below |
| Ledger order | *realm's own*, expressed through the positions it computes | Exact, because realm owns it outright |
| Stacking (mono occlusion, overlays) | `river_node_v1::place_top` / `place_bottom` / `place_above` / `place_below` | Exact |
| Stow | `river_window_v1::hide` / `show` — *rendering* state, so the window stays managed and stays in the ledger | Exact, and a closer match to `Orbit::stowed` than we expected |
| Focus | `river_seat_v1::focus_window` / `clear_focus` | Exact. Note `focus_exclusive` / `focus_non_exclusive` / `focus_none` are **events**, not requests: realm is told about exclusive focus, it does not grant it |
| 1px seams | `set_borders`, drawn by the compositor | Exact |
| Fullscreen | `fullscreen` / `exit_fullscreen` | Exact. Whether the bar draws over a fullscreen window is decided by the bar's chosen *layer*, not by node ordering: `river-layer-shell-v1` exposes no node and no ordering request at all |
| Window identity | `river_window_v1` `identifier` (up to 32 printable ASCII bytes) | **Requires a mapping.** `WindowOpened` carries `BackendWindowId`; realm-session restores or allocates a `WinId`, then calls `assign_window` before using that window. The never-reused property comes from realm's persisted counter, keyed by river's never-reused string |
| Workarea | `river_layer_shell_output_v1::non_exclusive_area` | Exact — arrives as an event. It is a free rectangle in global coordinates, *not* the `Workarea::new(w, h, top, bottom)` shape, so the backend converts |
| Atomic relayout | the `manage` **and** `render` sequences, in that order | Exact, but it is **two** phases and realm must respect the boundary — see below |

**A placement spans both phases.** `propose_dimensions` is window-management
state; `set_position` is *rendering* state. So a single `apply()` is not one
manage sequence: sizes go between `manage_start` and `manage_finish`, then
positions go after `render_start`.

The reason is a **data dependency**, not a prohibition. `propose_dimensions` is
manage-only; the resulting `dimensions` events arrive before `render_start`; and
a position cannot be finalised until then, because a window may not take the
size it was offered — the same quantisation problem flagged above. So realm keeps
positions in the render phase because that is where it first knows enough to
compute them.

(An earlier draft of this document asserted that a position submitted during the
manage phase raises `error::sequence_order`. The XML does not support that: the
`river_window_manager_v1` description permits rendering state to be modified
during *either* sequence and errors only outside both, and `set_position`'s own
text defers to that description by explicit cross-reference. The protocol is
arguably self-contradictory here; realm's behaviour is correct under either
reading, which is why the conclusion survived the correction.)

`place_*` orders the **render list**, not the ledger. The ledger is *layout*
order, which realm computes itself and expresses as positions; the `place_*`
requests exist for mono's occlusion stack and for overlay surfaces. Faithful
either way, but not for the reason a first reading suggests.

**The one genuinely approximate row.** `propose_dimensions` is a *proposal*: the
protocol explicitly anticipates clients quantising it, terminals to their cell
size being the named case. A terminal that rounds 700×580 down to 696×576 puts a
4×4 hole in a layout whose entire premise is exact tiling, and
`every_layout_tiles_exactly_for_every_plausible_size` would still pass while the
screen showed cracks — the test checks the projection, not what the client did
with it. river offers `set_content_clip_box`, which clips content to a rect and
draws borders around the intersection, so realm can propose at or above the tile
and clip to the exact rectangle. That is the plan; it is an M2 experiment with
its own guard, not a solved problem.

### What realm must implement, not merely call

Under river, a window manager is not only a client of the WM protocol. river's
`protocol/` directory holds **six** protocols, and five of them are obligations
realm must serve. The first is load-bearing and the last two are the difference
between a desktop and a demo on a laptop:

| Protocol | What realm owes it | Consequence if unimplemented |
|---|---|---|
| `river-layer-shell-v1` | Serve layer-shell on river's behalf | **The bar does not appear at all.** `wlr-layer-shell` works under river only if the window manager implements it |
| `river-xkb-bindings-v1` | The entire keymap, **and key repeat for bound keys** | No keybinding works. `ensure_next_key_eaten` and `ate_unbound_key` (on `river_xkb_bindings_seat_v1`, reached via `get_seat`) are purpose-built for chorded submaps, which is exactly realm's chord model. `stop_repeat` establishes that repeat for bound keys is the window manager's job, so realm owns a second timer — armed only between `pressed` and `released`, which is the justification ADR 0009's no-timers rule requires |
| `river-input-management-v1` | Seats, repeat rate, pointer config | No input configuration |
| `river-xkb-config-v1` | Keymap selection (`set_layout_by_name`), the `layout` event, caps and num lock | Layouts are frozen at whatever `XKB_DEFAULT_LAYOUT` was when river started, with no way to switch |
| `river-libinput-config-v1` | Tap-to-click, drag, natural scroll, accel profile and speed, click and scroll method, calibration | **A laptop has no tap-to-click and no way to get one.** Under river 0.4 there is no input config file — the window manager *is* the input configuration |

This is a materially larger phase-1 surface than "write a backend", and M2 is
scoped accordingly.

Two consequences worth stating plainly, because they cut both ways:

1. **`apply()` maps onto one manage/render transaction.** River applies sizes
   atomically between `manage_start` and `manage_finish`, then Realm applies
   positions and visibility before `render_finish`. The resulting relayout is
   never observed half-done.
2. **`realm-session` is now on the compositor's input path, with a hard liveness
   requirement.** Under niri, a crashed session daemon left a working if
   unmanaged desktop. Under river it leaves windows unplaced and keys dead, and
   the protocol warns that the compositor's input buffering is finite. River
   v0.4.8 queues 1024 seat events and then drops new input; it declares but does
   not post its `unresponsive` protocol error. **A stall is a session failure,
   not a slow frame.** Nothing in `realm-session` may block — not a theme apply,
   not a socket write to a wedged subscriber. This promotes the frame budgets
   in ARCHITECTURE §4 from performance goals to correctness requirements. See
   ADR 0013.

On stability: `river-window-management-v1` is **declared stable** as of river
0.4.0, with a forward-compatibility pledge to 1.0.0 — no `z` prefix, no
`unstable/` directory, interfaces already at v5. (An earlier draft of this
document called it registry-classified unstable. That was wrong: the
work-in-progress language came from a tracking issue that predates the release.)
The residual risk is not a protocol classification but trust in a single
maintainer of a pre-1.0 project, which is a different and smaller thing. realm
pins a tested river and treats a protocol bump as a tracked event.

ADR 0002 records the superseded plan to ship on niri, and the mapping table that
argued us out of it — worth reading before anyone proposes going back.

---

## 2. Theme generation contract — `realm-theme` (ADR 0005, ADR 0017)

One captured input set in, one sealed immutable generation selected for future
launches. [SPEC 0011](specs/0011-theme-activation-generations.md) supersedes the
former mutable target and reload interface for the supported path.

```rust
/// A file realm generates from the palette.
pub struct Template {
    /// Stable id, e.g. "gtk4", "foot", "yazi".
    pub id: &'static str,
    /// Source text with `{{ path.to.value }}` placeholders.
    pub source: &'static str,
    /// Normalized output path within a sealed generation.
    pub target: PathBuf,
    /// Catalogue metadata for a possible future generation-aware live upgrade.
    /// The supported apply path does not execute it.
    pub reload: Reload,
}

/// Canonical catalogue metadata. A current-pointer switch never executes this.
pub enum Reload {
    /// Catalogue declares that the consumer reads only at next start.
    None,
    /// Catalogue records a possible signal for a future owned-process protocol.
    Signal { process: &'static str, signal: i32 },
    /// Catalogue records a possible command for that future protocol.
    Command(Vec<String>),
    /// Catalogue identifies Realm-owned clients; apply sends no notification.
    RealmClients,
}

/// Publication result returned directly by the supported apply boundary.
pub enum GenerationPublicationOutcome {
    Committed(GenerationId),
    CommittedWithCleanupPending { generation: GenerationId, cause: String },
    OutcomeAmbiguous { candidate: GenerationId, cause: String },
}

/// A difference between candidate normalized outputs and a validated current
/// generation. Results are sorted by path and omit unchanged outputs.
pub enum ThemeOutputChange {
    Added(PathBuf),
    Removed(PathBuf),
    ByteDifferent(PathBuf),
}
```

The supported apply seam accepts safe input locators (or a test-only snapshot
builder), captures and renders them once under the exclusive generation lock,
and returns `GenerationPublicationOutcome` directly. It does not accept a
`Reloader`, inspect or write mutable target files, report `written` /
`unchanged` / `reloaded` lists, or notify a process. `Committed` and
`CommittedWithCleanupPending` identify the generation selected for future
launches; `OutcomeAmbiguous` is not reported as activated. Applying identical
inputs may publish a new generation; no no-op result is promised.
SPEC 0006 maps the variants deterministically: both committed variants are CLI
success (with a durable-selection cleanup warning for the latter), while
`OutcomeAmbiguous` is exit 6, names only an unconfirmed candidate, and cannot
trigger automatic recovery or retry.

The supported diff seam captures and renders the same inputs, then compares
their normalized output set with the manifest-listed bytes of a fully validated
`current` generation. It returns only sorted `Added`, `Removed`, and
`ByteDifferent` paths. It is read-only: no generated-root or lock
initialization, recovery, lease, GC, staging, publication, pointer replacement,
output write, signal, command, or session notification is permitted. Missing or
invalid current state is an error, not an empty baseline.

These names describe the public semantic contract, not a wire-compatibility
promise. Live upgrade and wire protocol design remain outside this interface.
Any future #22 upgrade must prove the selected generation of an owned process;
it cannot restore direct reload on pointer switch.

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

## 3. Bar render contract — `realm-bar` (ADR 0008, 0009)

The bar is a pure function of `RealmState` plus the palette. It owns no state
beyond its Wayland surface.

```rust
/// Draw one frame. Called only when the state or the palette changed.
fn render(state: &RealmState, palette: &Palette, probe: &Probe, canvas: &mut Pixmap) -> Damage;

/// The region that actually changed, so the compositor is handed a damage
/// rectangle rather than a whole-surface repaint.
pub struct Damage(Option<Rect>);
```

Rules, enforced by review and by the budgets in ARCHITECTURE.md §4:

1. **The bar owns no timer at all.** Every value it draws arrives in
   `RealmState`. Four of the mockup's modules — cpu, mem, gpu temperature and the
   `↑ 18k ↓ 1.2M` throughput half of net — are *rates over counters*, and the
   kernel exposes no event for those; no bar on any platform gets them without
   sampling. So the sampling lives in **one shared sampler in `realm-session`**,
   off the window-management event loop, and is the single documented exception
   to ADR 0009's no-timers rule. The bar stays a pure function of state, which
   is the property that actually mattered.
   The clock ticks to the next **minute** boundary, not every second: the design
   shows `14:32`, so 59 of every 60 wakeups would redraw nothing.
2. **No redraw when nothing changed.** `RealmState::renders_same_as` gates the
   frame before any drawing happens.
3. **Every glyph goes through `Probe::resolve`.** Drawing a raw `char` from the
   inventory bypasses the fallback contract and is how tofu ships.
4. **Damage, not repaint.** A clock tick must damage the clock, not the bar.

---

## 4. Control endpoint and client — `realm-control` (ADR 0004)

```rust
/// Validated Linux path capabilities. Their fields and owned descriptors are
/// private; accessors return display paths or borrows only.
pub struct RuntimeDir(/* absolute display path, retained fd, daemon euid */);
pub struct RealmDir(/* retained descriptor; exposes a borrowed fd */);
pub struct SocketEndpoint(/* exact ctl.sock display path + RealmDir */);
pub struct BoundControlEndpoint(/* non-listening fd + lock + path identity */);
pub struct ActiveControlListener(/* listening fd + lock + path identity */);
pub struct ControlServer(/* consumed listener + bounded connection state */);
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(u64); // private opaque value, never a raw fd
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ControlToken(u64); // private opaque poll-registration identity
pub struct PollInterest<'a> {
    pub token: ControlToken,
    pub fd: BorrowedFd<'a>,
    pub readable: bool,
    pub writable: bool,
}
pub struct ReadyEvent {
    pub token: ControlToken,
    pub readable: bool,
    pub writable: bool,
}
pub enum ControlAction {
    Request { connection: ConnectionId, request: Request },
}
pub enum ControlError {
    StaleConnection { connection: ConnectionId },
    ShuttingDown,
    OutboundFrameTooLarge { connections: Vec<ConnectionId> }, // at most 64, ascending
    PeerIo { connection: ConnectionId, source: std::io::Error },
    ListenerIo(std::io::Error),
    ResourceExhausted(std::io::Error),
}
pub struct ClientEndpoint(/* retained RuntimeDir for retry */);
pub struct Client(/* connected transport wrapper added by #41 */);
pub struct Subscription(/* consuming event iterator added by #41 */);

pub enum ClientPhase {
    Connect,
    HelloWrite,
    HelloRead,
    RequestWrite,
    ResponseRead,
    SubscribeWrite,
    InitialState,
    SubscriptionEvent,
}

pub enum ClientError {
    MissingRealm,
    Refused,
    Path(IpcPathError),
    VersionMismatch { client: u32, server: u32 },
    Timeout { phase: ClientPhase },
    FrameTooLarge { phase: ClientPhase },
    InvalidRequest,
    UnexpectedResponse { phase: ClientPhase },
    MalformedResponse { phase: ClientPhase },
    Eof { phase: ClientPhase },
    Io { phase: ClientPhase, source: std::io::Error },
}

pub enum IpcPathError {
    MissingRuntimeDir,
    UnsafeRuntimeDir,
    UnsafeRealmDirectory,
    UnsafeSocketEntry,
    EndpointInUse,
    Io(std::io::Error),
}

pub trait RuntimeDirResolver {
    fn resolve(&self) -> Result<RuntimeDir, IpcPathError>;
}

pub fn production_runtime_dir() -> Result<RuntimeDir, IpcPathError>;
pub fn test_runtime_dir(path: &Path) -> Result<RuntimeDir, IpcPathError>;

impl RuntimeDir {
    pub fn path(&self) -> &Path;
    pub fn prepare_server_endpoint(self) -> Result<SocketEndpoint, IpcPathError>;
    pub fn client_endpoint(self) -> ClientEndpoint;
}

impl RealmDir {
    /// Used later for descriptor-relative ledger.json work.
    pub fn as_fd(&self) -> BorrowedFd<'_>;
}

impl SocketEndpoint {
    pub fn path(&self) -> &Path;
    pub fn realm_dir(&self) -> &RealmDir;
    pub fn bind(self) -> Result<BoundControlEndpoint, IpcPathError>;
}

impl BoundControlEndpoint {
    pub fn endpoint(&self) -> &SocketEndpoint;
    pub fn realm_dir(&self) -> &RealmDir;
    pub fn activate(self) -> Result<ActiveControlListener, IpcPathError>;
}

impl ActiveControlListener {
    pub fn endpoint(&self) -> &SocketEndpoint;
    pub fn realm_dir(&self) -> &RealmDir;
    pub fn into_server(self, now: Instant) -> ControlServer;
}

impl ControlServer {
    pub fn poll_interests(&self) -> impl Iterator<Item = PollInterest<'_>>;
    pub fn next_deadline(&self) -> Option<Instant>;
    pub fn service_one(&mut self, now: Instant, ready: ReadyEvent)
        -> Result<Option<ControlAction>, ControlError>;
    pub fn expire(&mut self, now: Instant) -> Result<(), ControlError>;
    pub fn complete_request(&mut self, now: Instant, connection: ConnectionId,
        response: Response) -> Result<(), ControlError>;
    pub fn complete_subscribe(&mut self, now: Instant, connection: ConnectionId,
        state: RealmState) -> Result<(), ControlError>;
    pub fn publish_state(&mut self, now: Instant, state: RealmState)
        -> Result<(), ControlError>;
    pub fn begin_shutdown(&mut self, now: Instant);
    pub fn is_shutdown_complete(&self) -> bool;
}

/// A connection to realm-session, added with the #41 transport slice.
impl Client {
    pub fn request(&mut self, req: Request) -> Result<Response, ClientError>;
    /// Subscribe after a successful Hello; yields an immediate state snapshot
    /// and coalesced later changes until Shutdown/EOF/error. No further request
    /// is valid.
    pub fn subscribe(self) -> Result<Subscription, ClientError>;
}

impl ClientEndpoint {
    /// Make exactly one descriptor-relative attempt through a generated procfd
    /// bridge and complete Hello using the explicit client name.
    pub fn connect(&self, client: &str) -> Result<Client, ClientError>;
}

impl Iterator for Subscription {
    type Item = Result<Event, ClientError>;
}
```

`realm-control` is a new Linux-only shared workspace library; non-Linux
compilation fails explicitly. `realm-core` remains portable and owns only wire
values plus encode/decode/version. `BoundControlEndpoint` is not `Clone` and
intentionally has no `AsFd`; `activate(self)` is the only public path to
`listen(..., 64)`. It verifies `SO_ACCEPTCONN` before returning the not-`Clone`
active wrapper. The active wrapper is then consumed into `ControlServer` and
does not implement public `AsFd`. `poll_interests` exposes only temporary
borrowed fds paired with stable `ControlToken`s; callers copy token/readiness
and drop every borrow before `service_one`. Raw fds are never connection
identities.

Bound, active, and server ownership retain a separately opened singleton-lock
description and the no-follow pathname identity used for ownership-safe Drop
cleanup. That private
lock fd is opened independently with
`O_RDONLY | O_DIRECTORY | O_CLOEXEC` (or an exact equivalent), is checked for
`FD_CLOEXEC`, and is never returned by an accessor or `AsFd`. An exec-launched
client therefore cannot retain singleton ownership after the daemon exits.

Linux has no `bindat` or `connectat`. Bind, stale-probe connect, and
shared-client connect use only an internally generated
`/proc/self/fd/<realm-dir-fd>/ctl.sock` address; missing procfs fails closed.
The canonical address and a worst-case procfd address, including terminating
NULs, are checked against `sockaddr_un` and procfs accessibility is proved
before server-side filesystem mutation. The canonical public path is a
display/external-client path, not a shared-crate resolution route.
`ClientEndpoint` retains the validated runtime capability and reopens `realm`
relative to it on every single connection attempt; it never rereads
`XDG_RUNTIME_DIR` or creates the directory. An absent `realm` is the retryable
`MissingRealm` classification, not `UnsafeRealmDirectory`. `realmctl`, not
`ClientEndpoint`, owns the six absolute not-before retry targets from one fixed
driver start. A late retryable attempt skips elapsed sleep; attempts never
overlap or move backward. Server methods use caller-supplied
`std::time::Instant`; no production transport clock trait is part of this
interface. SPEC 0007 is the complete construction, singleton ownership,
transport, deadline, shutdown, and cleanup contract.

The wire types (`Request`, `Response`, `Event`, `RealmState`) already exist in
`realm-core::ipc` and `realm-core::state` and remain normative. `ControlAction`
emits every decoded non-Hello request without implementing session semantics;
the #38 adapter owns completions and authoritative state. The transport's
65,536-byte bound includes LF on input and output. One-peer oversized completion
closes that peer; oversized `publish_state` encodes once, closes all and only
current subscribers, and reports their at-most-64 stable ids in ascending
order. `service_one` applies due deadlines before socket I/O, disables reads
while application completion or ordinary output is pending, and gives
simultaneous subscriber readability priority over writability.
After an ordinary application response drains, including `Response::Error`,
only a read-open peer returns to `Ready`; a read-half-closed peer closes.

`begin_shutdown` is total and idempotent: its first call fixes the hard 100 ms
deadline, stops admission/actions, closes every non-subscriber, and leaves only
bounded subscriber drain. An unstarted initial State is preserved before
Shutdown; a partial current frame finishes alone. #38 drives interests and
expiry until `is_shutdown_complete()`, then calls River `exit_session`. No new
connection or action is admitted after shutdown starts. Later completion or
publication returns `ShuttingDown` without mutation, although an id already
stale before shutdown may remain `StaleConnection`. The `Client`,
`Subscription`, and `ControlServer` declarations describe the #41 transport
slice, not a claim that #218's endpoint slice implements it.

---

## 5. Activation lifecycle authority

Accepted [SPEC 0012](specs/0012-activation-launch-lifecycle.md) keeps lifecycle
selection, ownership evidence, lease transfer/release, durable state
transitions, and execution-gate authority private to `realm-theme`'s lifecycle
owner.  No `GenerationSelection` lifecycle-transfer method, lease reference, or
caller-constructed `LifecycleOwner` is a public interface.  The planned
fresh-Exec desktop-launch boundary is the consuming high-level facade constrained by
[SPEC 0013](specs/0013-truthful-fresh-desktop-exec.md): it accepts an immutable
admitted fresh-Exec plan and exposes neither an internal lifecycle capability
nor a public wire protocol.  Public request/response/history design remains
SPEC 0006/#117.  The private implementation still has the accepted
consume-on-transfer, no-drop-release, and proof-before-release obligations;
their concrete Rust types are not external compatibility interfaces.

---

## 6. What is deliberately *not* an interface

- **The ledger.** There is one implementation and there will only ever be one.
  Making it a trait would invite a second source of truth, which is the exact
  failure ADR 0001 exists to prevent.
- **The layout projection.** Same reason: layouts are an enum with a pure
  function, not a plugin surface. A layout that cannot be expressed as
  `fn(&Ledger, Workarea) -> Vec<Placement>` is a layout realm does not want.
