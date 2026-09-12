use std::{
    ffi::OsStr,
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use calloop::{
    channel::{self, Event as ChannelEvent},
    EventLoop,
};
use calloop_wayland_source::WaylandSource;
use realm_bar::{
    surface_config, Anchor as RealmAnchor, BarRenderer, Frame, Layer as RealmLayer, SurfaceCommand,
    SurfaceKind, SurfaceLifecycle,
};
use realm_control::{
    production_runtime_dir, ClientEndpoint, ClientError, ClientPhase, Subscription,
};
use realm_core::{
    glyphs::Probe,
    ipc::{Event, Request, Response},
    keys::Keymap,
    palette::Palette,
    state::RealmState,
};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::Buffer, slot::SlotPool, Shm, ShmHandler},
};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, QueueHandle,
};

enum ControlMessage {
    Event(Event),
    Keymap(Keymap),
    Failed(String),
}

const CONTROL_RETRY_INTERVAL: Duration = Duration::from_millis(250);

struct ControlConnection {
    keymap: Keymap,
    subscription: Subscription,
}

enum ControlConnectError {
    Retry(ClientError),
    Fatal(anyhow::Error),
}

struct ControlWatch {
    fd: std::os::fd::OwnedFd,
    expected_name: &'static OsStr,
}

struct Surface {
    kind: SurfaceKind,
    layer: LayerSurface,
    pool: SlotPool,
    buffers: Vec<Buffer>,
    logical_width: u32,
    logical_height: u32,
    scale: u32,
    configured: bool,
    pending: bool,
    pending_frame: Option<Frame>,
}

struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    palette: Palette,
    keymap: Keymap,
    state: Option<RealmState>,
    lifecycle: SurfaceLifecycle,
    renderer: BarRenderer,
    probe: Probe,
    surfaces: Vec<Surface>,
    exit: bool,
    failure: Option<String>,
}

pub(super) fn run(palette: Palette) -> Result<()> {
    let runtime = production_runtime_dir().context("resolve XDG_RUNTIME_DIR for realm-control")?;
    let runtime_path = runtime.path().to_owned();
    let endpoint = runtime.client_endpoint();
    let ControlConnection {
        keymap,
        subscription,
    } = connect_control(&endpoint, &runtime_path)?;

    let connection = Connection::connect_to_env().context("connect to Wayland display")?;
    let (globals, event_queue) =
        registry_queue_init(&connection).context("enumerate Wayland globals")?;
    let queue_handle = event_queue.handle();
    let compositor = CompositorState::bind(&globals, &queue_handle)
        .context("Wayland server does not advertise wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &queue_handle)
        .context("Wayland server does not advertise wlr-layer-shell")?;
    let shm =
        Shm::bind(&globals, &queue_handle).context("Wayland server does not advertise wl_shm")?;
    let renderer = BarRenderer::new(&palette).context("initialize bar font stack")?;
    let probe = renderer.probe();
    tracing::info!("{}", probe.summary());

    let mut app = App {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &queue_handle),
        compositor,
        layer_shell,
        shm,
        palette,
        keymap,
        state: None,
        lifecycle: SurfaceLifecycle::new(1, 1, 1),
        renderer,
        probe,
        surfaces: Vec::new(),
        exit: false,
        failure: None,
    };

    let mut event_loop: EventLoop<App> = EventLoop::try_new().context("create event loop")?;
    WaylandSource::new(connection, event_queue)
        .insert(event_loop.handle())
        .map_err(|_| anyhow::anyhow!("insert Wayland event source"))?;

    let (sender, receiver) = channel::sync_channel::<ControlMessage>(1);
    let qh = queue_handle.clone();
    event_loop
        .handle()
        .insert_source(receiver, move |event, _, app| match event {
            ChannelEvent::Msg(ControlMessage::Event(Event::State(state))) => {
                if let Err(error) = app.apply_state(*state, &qh) {
                    tracing::error!(%error, "could not apply Realm state");
                    app.failure = Some(error.to_string());
                    app.exit = true;
                }
            }
            ChannelEvent::Msg(ControlMessage::Keymap(keymap)) => {
                app.keymap = keymap;
                if let Err(error) = app.apply_commands(
                    vec![
                        SurfaceCommand::Draw(SurfaceKind::WhichKey),
                        SurfaceCommand::Draw(SurfaceKind::Grimoire),
                    ],
                    &qh,
                ) {
                    tracing::error!(%error, "could not redraw reconnected key discovery surfaces");
                    app.failure = Some(error.to_string());
                    app.exit = true;
                }
            }
            ChannelEvent::Msg(ControlMessage::Event(Event::Shutdown)) => {
                app.exit = true;
            }
            ChannelEvent::Closed => {
                app.failure = Some("realm-control subscription ended without shutdown".into());
                app.exit = true;
            }
            ChannelEvent::Msg(ControlMessage::Failed(error)) => {
                tracing::error!(error, "realm-control subscription failed");
                app.failure = Some(error);
                app.exit = true;
            }
        })
        .map_err(|_| anyhow::anyhow!("insert realm-control event source"))?;

    thread::Builder::new()
        .name("realm-bar-control".into())
        .spawn(move || {
            control_worker(endpoint, runtime_path, subscription, sender);
        })
        .context("start realm-control subscription thread")?;

    while !app.exit {
        event_loop
            .dispatch(None, &mut app)
            .context("dispatch bar event loop")?;
    }
    if let Some(failure) = app.failure {
        bail!(failure);
    }
    Ok(())
}

fn control_worker(
    endpoint: ClientEndpoint,
    runtime_path: PathBuf,
    mut subscription: Subscription,
    sender: channel::SyncSender<ControlMessage>,
) {
    loop {
        let mut disconnected = None;
        for event in subscription.by_ref() {
            match event {
                Ok(Event::State(state)) => {
                    if sender
                        .send(ControlMessage::Event(Event::State(state)))
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(Event::Shutdown) => {
                    let _ = sender.send(ControlMessage::Event(Event::Shutdown));
                    return;
                }
                Err(error) if subscription_reconnectable(&error) => {
                    disconnected = Some(error.to_string());
                    break;
                }
                Err(error) => {
                    let _ = sender.send(ControlMessage::Failed(error.to_string()));
                    return;
                }
            }
        }
        if let Some(error) = disconnected {
            tracing::warn!(error, "realm-control disconnected; awaiting replacement");
        }

        match connect_control(&endpoint, &runtime_path) {
            Ok(connection) => {
                if sender
                    .send(ControlMessage::Keymap(connection.keymap))
                    .is_err()
                {
                    return;
                }
                subscription = connection.subscription;
            }
            Err(error) => {
                let _ = sender.send(ControlMessage::Failed(error.to_string()));
                return;
            }
        }
    }
}

fn connect_control(endpoint: &ClientEndpoint, runtime_path: &Path) -> Result<ControlConnection> {
    loop {
        // Arm the filesystem notification before attempting the connection so
        // creation/replacement cannot be lost between a failed attempt and wait.
        let watch = ControlWatch::new(runtime_path).context("watch realm-control endpoint")?;
        match try_connect_control(endpoint) {
            Ok(connection) => return Ok(connection),
            Err(ControlConnectError::Fatal(error)) => return Err(error),
            Err(ControlConnectError::Retry(error)) => {
                tracing::info!(%error, "realm-control unavailable; waiting for endpoint change");
                watch
                    .wait()
                    .context("wait for realm-control endpoint change")?;
            }
        }
    }
}

fn try_connect_control(
    endpoint: &ClientEndpoint,
) -> std::result::Result<ControlConnection, ControlConnectError> {
    let mut client = endpoint
        .connect("realm-bar")
        .map_err(classify_control_error)?;
    let keymap = match client
        .request(Request::GetKeymap)
        .map_err(classify_control_error)?
    {
        Response::Keymap(keymap) => *keymap,
        Response::Error { message } => {
            return Err(ControlConnectError::Fatal(anyhow::anyhow!(
                "session refused keymap request: {message}"
            )));
        }
        response => {
            return Err(ControlConnectError::Fatal(anyhow::anyhow!(
                "unexpected keymap response: {response:?}"
            )));
        }
    };
    let subscription = client.subscribe().map_err(classify_control_error)?;
    Ok(ControlConnection {
        keymap,
        subscription,
    })
}

fn classify_control_error(error: ClientError) -> ControlConnectError {
    if reconnectable(&error) {
        ControlConnectError::Retry(error)
    } else {
        ControlConnectError::Fatal(error.into())
    }
}

fn reconnectable(error: &ClientError) -> bool {
    error.is_retryable()
}

fn subscription_reconnectable(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Eof {
            phase: ClientPhase::SubscriptionEvent
        } | ClientError::Io {
            phase: ClientPhase::SubscriptionEvent,
            ..
        }
    )
}

impl ControlWatch {
    fn new(runtime_path: &Path) -> Result<Self> {
        let realm_path = runtime_path.join("realm");
        let (path, expected_name): (&Path, &'static OsStr) = if realm_path
            .symlink_metadata()
            .is_ok_and(|meta| meta.is_dir())
        {
            (&realm_path, OsStr::new("ctl.sock"))
        } else {
            (runtime_path, OsStr::new("realm"))
        };
        let fd = inotify::init(CreateFlags::CLOEXEC | CreateFlags::NONBLOCK)
            .context("create inotify descriptor")?;
        inotify::add_watch(
            &fd,
            path,
            WatchFlags::CREATE
                | WatchFlags::DELETE
                | WatchFlags::ATTRIB
                | WatchFlags::MOVED_FROM
                | WatchFlags::MOVED_TO
                | WatchFlags::DELETE_SELF
                | WatchFlags::MOVE_SELF
                | WatchFlags::ONLYDIR
                | WatchFlags::DONT_FOLLOW,
        )
        .with_context(|| format!("watch {}", path.display()))?;
        Ok(Self { fd, expected_name })
    }

    fn wait(self) -> Result<()> {
        let deadline = Instant::now() + CONTROL_RETRY_INTERVAL;
        let mut storage = [MaybeUninit::uninit(); 512];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(());
            }
            let timeout = duration_timespec(remaining);
            let mut fds = [PollFd::new(&self.fd, PollFlags::IN)];
            if poll(&mut fds, Some(&timeout)).context("poll inotify descriptor")? == 0 {
                return Ok(());
            }

            let mut events = inotify::Reader::new(&self.fd, &mut storage);
            loop {
                let event = match events.next() {
                    Ok(event) => event,
                    Err(rustix::io::Errno::AGAIN) => break,
                    Err(error) => return Err(error).context("read inotify event"),
                };
                let self_changed = event.events().intersects(
                    ReadFlags::DELETE_SELF
                        | ReadFlags::MOVE_SELF
                        | ReadFlags::IGNORED
                        | ReadFlags::QUEUE_OVERFLOW,
                );
                let expected_changed = event
                    .file_name()
                    .is_some_and(|name| name.to_bytes() == self.expected_name.as_bytes());
                if self_changed || expected_changed {
                    return Ok(());
                }
            }
        }
    }
}

fn duration_timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: duration.as_secs().try_into().unwrap_or(i64::MAX),
        tv_nsec: duration.subsec_nanos().into(),
    }
}

impl App {
    fn apply_state(&mut self, state: RealmState, qh: &QueueHandle<Self>) -> Result<()> {
        let commands = self.lifecycle.update(&state, &self.palette);
        self.state = Some(state);
        self.apply_commands(commands, qh)
    }

    fn apply_commands(
        &mut self,
        commands: Vec<SurfaceCommand>,
        qh: &QueueHandle<Self>,
    ) -> Result<()> {
        for command in commands {
            match command {
                SurfaceCommand::Create(kind) => self.create_surface(kind, qh)?,
                SurfaceCommand::Destroy(kind) => self.destroy_surface(kind),
                SurfaceCommand::Draw(kind) => self.draw_surface(kind, qh)?,
            }
        }
        Ok(())
    }

    fn create_surface(&mut self, kind: SurfaceKind, qh: &QueueHandle<Self>) -> Result<()> {
        if self.surfaces.iter().any(|surface| surface.kind == kind) {
            return Ok(());
        }
        let config = surface_config(kind, &self.palette, 1, 1, 1);
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            match config.layer {
                RealmLayer::Bottom => Layer::Bottom,
                RealmLayer::Overlay => Layer::Overlay,
            },
            Some(match kind {
                SurfaceKind::Bar => "realm-bar",
                SurfaceKind::WhichKey => "realm-which-key",
                SurfaceKind::Grimoire => "realm-grimoire",
            }),
            None,
        );
        layer.set_anchor(to_wayland_anchor(config.anchors));
        layer.set_size(config.logical_size.0, config.logical_size.1);
        layer.set_exclusive_zone(config.exclusive_zone);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        let pool = SlotPool::new(1, &self.shm).context("create wl_shm slot pool")?;
        self.surfaces.push(Surface {
            kind,
            layer,
            pool,
            buffers: Vec::with_capacity(2),
            logical_width: config.logical_size.0,
            logical_height: config.logical_size.1,
            scale: 1,
            configured: false,
            pending: true,
            pending_frame: None,
        });
        Ok(())
    }

    fn destroy_surface(&mut self, kind: SurfaceKind) {
        if let Some(index) = self
            .surfaces
            .iter()
            .position(|surface| surface.kind == kind)
        {
            self.surfaces.remove(index);
        }
    }

    fn draw_surface(&mut self, kind: SurfaceKind, qh: &QueueHandle<Self>) -> Result<()> {
        let Some(index) = self
            .surfaces
            .iter()
            .position(|surface| surface.kind == kind)
        else {
            return Ok(());
        };
        if !self.surfaces[index].configured {
            self.surfaces[index].pending = true;
            return Ok(());
        }
        let Some(state) = self.state.as_ref() else {
            return Ok(());
        };
        let surface = &self.surfaces[index];
        let frame = match kind {
            SurfaceKind::Bar => self.renderer.render_bar(
                state,
                &self.palette,
                &self.probe,
                surface.logical_width,
                surface.scale,
            ),
            SurfaceKind::WhichKey => self.renderer.render_which_key(
                &self.keymap,
                &self.palette,
                &self.probe,
                surface.logical_width,
                surface.scale,
            ),
            SurfaceKind::Grimoire => self.renderer.render_grimoire(
                &self.keymap,
                &self.palette,
                &self.probe,
                surface.logical_width,
                surface.logical_height,
                surface.scale,
            ),
        };
        self.commit_frame(index, frame, qh)
    }

    fn commit_frame(&mut self, index: usize, frame: Frame, qh: &QueueHandle<Self>) -> Result<()> {
        let Some(damage) = frame.damage.rect() else {
            self.surfaces[index].pending = false;
            self.surfaces[index].pending_frame = None;
            return Ok(());
        };
        let width = frame.width.saturating_mul(frame.scale).max(1);
        let height = frame.height.saturating_mul(frame.scale).max(1);
        let stride = width.saturating_mul(4) as i32;
        let surface = &mut self.surfaces[index];

        let mut stale_index = 0;
        while stale_index < surface.buffers.len() {
            let stale = surface.buffers[stale_index].height() != height as i32
                || surface.buffers[stale_index].stride() != stride;
            let released = surface.buffers[stale_index]
                .canvas(&mut surface.pool)
                .is_some();
            if stale && released {
                surface.buffers.remove(stale_index);
            } else {
                stale_index += 1;
            }
        }

        let mut available = None;
        for (buffer_index, buffer) in surface.buffers.iter().enumerate() {
            if buffer.height() == height as i32
                && buffer.stride() == stride
                && buffer.canvas(&mut surface.pool).is_some()
            {
                available = Some(buffer_index);
                break;
            }
        }

        if available.is_none() {
            let matching = surface
                .buffers
                .iter()
                .filter(|buffer| buffer.height() == height as i32 && buffer.stride() == stride)
                .count();
            if matching < 2 {
                let (buffer, canvas) = surface
                    .pool
                    .create_buffer(
                        width as i32,
                        height as i32,
                        stride,
                        wl_shm::Format::Argb8888,
                    )
                    .context("allocate wl_shm buffer")?;
                rgba_to_argb8888(&frame.pixels, canvas);
                surface.buffers.push(buffer);
                available = Some(surface.buffers.len() - 1);
            }
        }

        let Some(buffer_index) = available else {
            surface.pending = true;
            surface.pending_frame = Some(frame);
            surface
                .layer
                .wl_surface()
                .frame(qh, FrameCallbackData(surface.layer.wl_surface().clone()));
            surface.layer.commit();
            return Ok(());
        };
        if let Some(canvas) = surface.buffers[buffer_index].canvas(&mut surface.pool) {
            rgba_to_argb8888(&frame.pixels, canvas);
        }
        surface.buffers[buffer_index]
            .attach_to(surface.layer.wl_surface())
            .context("attach wl_shm buffer")?;
        let damage = damage.scaled(frame.scale);
        surface.layer.wl_surface().damage_buffer(
            damage.x,
            damage.y,
            damage.width as i32,
            damage.height as i32,
        );
        surface.layer.commit();
        surface.pending = false;
        surface.pending_frame = None;
        let committed_kind = surface.kind;
        if committed_kind == SurfaceKind::Bar {
            self.renderer.commit_bar_frame();
        }
        Ok(())
    }

    fn retry_surface(&mut self, kind: SurfaceKind, qh: &QueueHandle<Self>) -> Result<()> {
        let Some(index) = self
            .surfaces
            .iter()
            .position(|surface| surface.kind == kind)
        else {
            return Ok(());
        };
        if let Some(frame) = self.surfaces[index].pending_frame.take() {
            self.commit_frame(index, frame, qh)
        } else {
            self.draw_surface(kind, qh)
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        wl_surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if let Some(surface) = self
            .surfaces
            .iter_mut()
            .find(|surface| surface.layer.wl_surface() == wl_surface)
        {
            surface.scale = new_factor.max(1) as u32;
            surface
                .layer
                .wl_surface()
                .set_buffer_scale(new_factor.max(1));
            surface.pending = true;
            let kind = surface.kind;
            if let Err(error) = self.draw_surface(kind, qh) {
                tracing::error!(%error, "could not redraw scale-changed surface");
                self.failure = Some(error.to_string());
                self.exit = true;
            }
        }
    }

    fn transform_changed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        wl_surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        let kind = self
            .surfaces
            .iter()
            .find(|surface| surface.layer.wl_surface() == wl_surface && surface.pending)
            .map(|surface| surface.kind);
        if let Some(kind) = kind {
            if let Err(error) = self.retry_surface(kind, qh) {
                tracing::error!(%error, "could not redraw pending surface");
                self.failure = Some(error.to_string());
                self.exit = true;
            }
        }
    }

    fn surface_enter(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _connection: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(index) = self
            .surfaces
            .iter()
            .position(|surface| &surface.layer == layer)
        {
            let kind = self.surfaces[index].kind;
            self.surfaces.remove(index);
            self.failure = Some(format!("compositor closed {kind:?} layer surface"));
            self.exit = true;
        }
    }

    fn configure(
        &mut self,
        _connection: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self
            .surfaces
            .iter()
            .position(|surface| &surface.layer == layer)
        else {
            return;
        };
        let surface = &mut self.surfaces[index];
        if configure.new_size.0 != 0 {
            surface.logical_width = configure.new_size.0;
        }
        if configure.new_size.1 != 0 {
            surface.logical_height = configure.new_size.1;
        }
        surface.configured = surface.logical_width > 0 && surface.logical_height > 0;
        surface.pending = true;
        let kind = surface.kind;
        if let Err(error) = self.draw_surface(kind, qh) {
            tracing::error!(%error, "could not draw configured surface");
            self.failure = Some(error.to_string());
            self.exit = true;
        }
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _connection: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState,];
}

fn to_wayland_anchor(anchor: RealmAnchor) -> Anchor {
    let mut result = Anchor::empty();
    if anchor.contains(RealmAnchor::TOP) {
        result |= Anchor::TOP;
    }
    if anchor.contains(RealmAnchor::BOTTOM) {
        result |= Anchor::BOTTOM;
    }
    if anchor.contains(RealmAnchor::LEFT) {
        result |= Anchor::LEFT;
    }
    if anchor.contains(RealmAnchor::RIGHT) {
        result |= Anchor::RIGHT;
    }
    result
}

fn rgba_to_argb8888(source: &[u8], destination: &mut [u8]) {
    for (rgba, argb) in source
        .as_chunks::<4>()
        .0
        .iter()
        .zip(destination.as_chunks_mut::<4>().0.iter_mut())
    {
        argb[0] = rgba[2];
        argb[1] = rgba[1];
        argb[2] = rgba[0];
        argb[3] = rgba[3];
    }
}

smithay_client_toolkit::delegate_registry!(App);
smithay_client_toolkit::delegate_dispatch2!(App);

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::MetadataExt,
        os::unix::net::UnixStream,
        sync::mpsc,
        time::{Duration, Instant},
    };

    use realm_control::{ClientError, ClientPhase};
    use rustix::net::{
        bind, listen, socket_with, AddressFamily, SocketAddrUnix, SocketFlags, SocketType,
    };

    use super::{reconnectable, subscription_reconnectable, ControlWatch, CONTROL_RETRY_INTERVAL};

    #[test]
    fn fresh_handshake_retries_only_missing_or_refused_endpoints() {
        assert!(reconnectable(&ClientError::MissingRealm));
        assert!(reconnectable(&ClientError::Refused));
        assert!(!reconnectable(&ClientError::Eof {
            phase: ClientPhase::SubscriptionEvent,
        }));
        assert!(!reconnectable(&ClientError::Timeout {
            phase: ClientPhase::HelloRead,
        }));
        assert!(!reconnectable(&ClientError::VersionMismatch {
            client: 1,
            server: 2,
        }));
    }

    #[test]
    fn established_subscription_reconnects_only_on_disconnect() {
        assert!(subscription_reconnectable(&ClientError::Eof {
            phase: ClientPhase::SubscriptionEvent,
        }));
        assert!(subscription_reconnectable(&ClientError::Io {
            phase: ClientPhase::SubscriptionEvent,
            source: std::io::Error::from(std::io::ErrorKind::ConnectionReset),
        }));
        assert!(!subscription_reconnectable(
            &ClientError::MalformedResponse {
                phase: ClientPhase::SubscriptionEvent,
            }
        ));
        assert!(!subscription_reconnectable(&ClientError::Timeout {
            phase: ClientPhase::SubscriptionEvent,
        }));
    }

    #[test]
    fn control_watch_wakes_when_the_realm_directory_appears() {
        let runtime = std::env::temp_dir().join(format!(
            "realm-bar-watch-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir(&runtime).unwrap();
        let watch = ControlWatch::new(&runtime).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let waiter = std::thread::spawn(move || sender.send(watch.wait()).unwrap());

        fs::create_dir(runtime.join("realm")).unwrap();
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("inotify wakeup")
            .unwrap();
        waiter.join().unwrap();
        fs::remove_dir(runtime.join("realm")).unwrap();
        fs::remove_dir(runtime).unwrap();
    }

    #[test]
    fn retry_wakes_when_a_prebound_socket_starts_listening_on_the_same_inode() {
        let runtime = std::env::temp_dir().join(format!(
            "realm-bar-prebound-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let realm = runtime.join("realm");
        let socket_path = realm.join("ctl.sock");
        fs::create_dir(&runtime).unwrap();
        fs::create_dir(&realm).unwrap();
        let server = socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )
        .unwrap();
        bind(&server, &SocketAddrUnix::new(&socket_path).unwrap()).unwrap();
        let inode = fs::symlink_metadata(&socket_path).unwrap().ino();
        let watch = ControlWatch::new(&runtime).unwrap();

        assert_eq!(
            UnixStream::connect(&socket_path).unwrap_err().kind(),
            std::io::ErrorKind::ConnectionRefused
        );
        let started = Instant::now();
        watch.wait().unwrap();
        assert!(started.elapsed() >= CONTROL_RETRY_INTERVAL / 2);

        listen(&server, 8).unwrap();
        assert_eq!(fs::symlink_metadata(&socket_path).unwrap().ino(), inode);
        let connection = UnixStream::connect(&socket_path).unwrap();
        drop(connection);
        drop(server);
        fs::remove_file(socket_path).unwrap();
        fs::remove_dir(realm).unwrap();
        fs::remove_dir(runtime).unwrap();
    }
}
