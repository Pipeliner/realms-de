//! Production River v0.4.8 window-management backend.

use std::io;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;

use realm_core::ipc::Capabilities;
use rustix::event::{poll, PollFd, PollFlags};
use wayland_backend::client::{Backend, DispatchOne, FlushOnce, ObjectData, ObjectId, ReadOnce};
use wayland_backend::protocol::Message;
use wayland_client::protocol::{wl_callback, wl_display, wl_registry};
use wayland_client::{Connection, Proxy};

use super::river_protocols::{
    river_input_manager_v1::RiverInputManagerV1, river_layer_shell_v1::RiverLayerShellV1,
    river_libinput_config_v1::RiverLibinputConfigV1, river_window_manager_v1::RiverWindowManagerV1,
    river_xkb_bindings_v1::RiverXkbBindingsV1,
};
use super::{BackendError, BackendResult};

const INCOMING_CAPACITY: usize = 512;

#[derive(Debug)]
enum Incoming {
    Global {
        name: u32,
        interface: String,
        version: u32,
    },
    GlobalRemoved(u32),
    SyncDone,
    Malformed,
}

#[derive(Debug, Clone, Copy)]
enum ObjectKind {
    Registry,
    Callback,
    NoEvents,
}

#[derive(Debug)]
struct DirectData {
    kind: ObjectKind,
    incoming: SyncSender<Incoming>,
}

impl DirectData {
    fn new(kind: ObjectKind, incoming: &SyncSender<Incoming>) -> Arc<Self> {
        Arc::new(Self {
            kind,
            incoming: incoming.clone(),
        })
    }

    fn emit(&self, event: Incoming) {
        match self.incoming.try_send(event) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                let _ = self.incoming.try_send(Incoming::Malformed);
            }
        }
    }
}

impl ObjectData for DirectData {
    fn event(
        self: Arc<Self>,
        backend: &Backend,
        message: Message<ObjectId, OwnedFd>,
    ) -> Option<Arc<dyn ObjectData>> {
        let connection = Connection::from_backend(backend.clone());
        match self.kind {
            ObjectKind::Registry => {
                match wl_registry::WlRegistry::parse_event(&connection, message) {
                    Ok((
                        _,
                        wl_registry::Event::Global {
                            name,
                            interface,
                            version,
                        },
                    )) => {
                        self.emit(Incoming::Global {
                            name,
                            interface,
                            version,
                        });
                    }
                    Ok((_, wl_registry::Event::GlobalRemove { name })) => {
                        self.emit(Incoming::GlobalRemoved(name));
                    }
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::Callback => {
                match wl_callback::WlCallback::parse_event(&connection, message) {
                    Ok((_, wl_callback::Event::Done { .. })) => self.emit(Incoming::SyncDone),
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::NoEvents => self.emit(Incoming::Malformed),
        }
        None
    }

    fn destroyed(&self, _object_id: ObjectId) {}
}

#[derive(Debug, Clone)]
struct AdvertisedGlobal {
    name: u32,
    version: u32,
}

#[derive(Debug, Default)]
struct Globals {
    window_manager: Option<AdvertisedGlobal>,
    xkb_bindings: Option<AdvertisedGlobal>,
    layer_shell: Option<AdvertisedGlobal>,
    input_manager: Option<AdvertisedGlobal>,
    libinput_config: Option<AdvertisedGlobal>,
}

/// A live connection to River's Wayland display.
#[derive(Debug)]
pub struct RiverBackend {
    connection: Connection,
    transport: Backend,
    incoming_tx: SyncSender<Incoming>,
    incoming_rx: Receiver<Incoming>,
    registry: Option<wl_registry::WlRegistry>,
    window_manager: Option<RiverWindowManagerV1>,
    xkb_bindings: Option<RiverXkbBindingsV1>,
    layer_shell: Option<RiverLayerShellV1>,
    input_manager: Option<RiverInputManagerV1>,
    libinput_config: Option<RiverLibinputConfigV1>,
    connected: bool,
}

impl RiverBackend {
    /// Connect to the Wayland display selected by `WAYLAND_SOCKET` or
    /// `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`.
    pub fn from_env() -> BackendResult<Self> {
        let connection =
            Connection::connect_to_env().map_err(|error| BackendError::Unavailable {
                message: format!("could not connect to River Wayland display: {error}"),
            })?;
        Ok(Self::from_connection(connection))
    }

    /// Construct a River backend from an already-connected Wayland socket.
    ///
    /// This is useful for embedding Realm under a compositor whose connection
    /// was established by a supervisor and for real-socket protocol fixtures.
    pub fn from_socket(socket: UnixStream) -> BackendResult<Self> {
        let connection = Connection::from_socket(socket).map_err(|error| BackendError::Io {
            message: format!("could not initialize Wayland transport: {error}"),
        })?;
        Ok(Self::from_connection(connection))
    }

    fn from_connection(connection: Connection) -> Self {
        let transport = connection.backend();
        let (incoming_tx, incoming_rx) = mpsc::sync_channel(INCOMING_CAPACITY);
        Self {
            connection,
            transport,
            incoming_tx,
            incoming_rx,
            registry: None,
            window_manager: None,
            xkb_bindings: None,
            layer_shell: None,
            input_manager: None,
            libinput_config: None,
            connected: false,
        }
    }

    /// Human-readable compositor backend name.
    pub fn name(&self) -> &str {
        "river-v0.4.8"
    }

    /// Wayland connection descriptor used by the outer poll loop.
    pub fn event_fd(&self) -> BorrowedFd<'_> {
        self.transport.poll_fd()
    }

    /// Discover River's required globals, bind the companion protocols, settle
    /// the two input-policy fences, and bind window management last.
    pub fn connect(&mut self) -> BackendResult<Capabilities> {
        if self.connected {
            return Ok(full_capabilities());
        }

        let display = self.connection.display();
        let registry = display
            .send_constructor::<wl_registry::WlRegistry>(
                wl_display::Request::GetRegistry {},
                DirectData::new(ObjectKind::Registry, &self.incoming_tx),
            )
            .map_err(invalid_id)?;
        self.registry = Some(registry.clone());
        let discovery = self.sync()?;
        let mut globals = Globals::default();
        self.pump_until_sync(discovery, |incoming| match incoming {
            Incoming::Global {
                name,
                interface,
                version,
            } => globals.record(name, interface, version),
            Incoming::GlobalRemoved(name) => globals.remove(name),
            Incoming::SyncDone => Ok(true),
            Incoming::Malformed => Err(protocol_error()),
        })?;
        globals.validate()?;

        self.layer_shell = Some(bind_global::<RiverLayerShellV1>(
            &registry,
            globals.layer_shell.as_ref().unwrap(),
            1,
            &self.incoming_tx,
        )?);
        self.xkb_bindings = Some(bind_global::<RiverXkbBindingsV1>(
            &registry,
            globals.xkb_bindings.as_ref().unwrap(),
            3,
            &self.incoming_tx,
        )?);
        self.input_manager = Some(bind_global::<RiverInputManagerV1>(
            &registry,
            globals.input_manager.as_ref().unwrap(),
            2,
            &self.incoming_tx,
        )?);
        self.libinput_config = Some(bind_global::<RiverLibinputConfigV1>(
            &registry,
            globals.libinput_config.as_ref().unwrap(),
            2,
            &self.incoming_tx,
        )?);

        let input_discovery = self.sync()?;
        self.pump_until_sync(input_discovery, |incoming| match incoming {
            Incoming::SyncDone => Ok(true),
            Incoming::Global { .. } | Incoming::GlobalRemoved(_) => Ok(false),
            Incoming::Malformed => Err(protocol_error()),
        })?;
        let input_results = self.sync()?;
        self.pump_until_sync(input_results, |incoming| match incoming {
            Incoming::SyncDone => Ok(true),
            Incoming::Global { .. } | Incoming::GlobalRemoved(_) => Ok(false),
            Incoming::Malformed => Err(protocol_error()),
        })?;

        self.window_manager = Some(bind_global::<RiverWindowManagerV1>(
            &registry,
            globals.window_manager.as_ref().unwrap(),
            5,
            &self.incoming_tx,
        )?);
        self.flush_blocking()?;
        self.connected = true;
        Ok(full_capabilities())
    }

    fn sync(&self) -> BackendResult<wl_callback::WlCallback> {
        self.connection
            .display()
            .send_constructor::<wl_callback::WlCallback>(
                wl_display::Request::Sync {},
                DirectData::new(ObjectKind::Callback, &self.incoming_tx),
            )
            .map_err(invalid_id)
    }

    fn pump_until_sync(
        &self,
        _callback: wl_callback::WlCallback,
        mut consume: impl FnMut(Incoming) -> BackendResult<bool>,
    ) -> BackendResult<()> {
        loop {
            match self.transport.flush_once().map_err(wayland_error)? {
                FlushOnce::Complete => {}
                FlushOnce::Progress { .. } => continue,
                FlushOnce::WouldBlock => self.wait(PollFlags::OUT)?,
            }

            match self.incoming_rx.try_recv() {
                Ok(incoming) => {
                    if consume(incoming)? {
                        return Ok(());
                    }
                    continue;
                }
                Err(TryRecvError::Disconnected) => return Err(protocol_error()),
                Err(TryRecvError::Empty) => {}
            }

            match self
                .transport
                .dispatch_one_pending()
                .map_err(wayland_error)?
            {
                DispatchOne::Dispatched => continue,
                DispatchOne::PreparedReadLive => return Err(protocol_error()),
                DispatchOne::NeedRead => {}
            }

            let guard = self
                .transport
                .prepare_read_bounded()
                .ok_or_else(protocol_error)?;
            self.wait(PollFlags::IN)?;
            match guard.read_once().map_err(wayland_error)? {
                ReadOnce::Read { .. } | ReadOnce::WouldBlock => {}
            }
        }
    }

    fn wait(&self, interest: PollFlags) -> BackendResult<()> {
        let fd = self.transport.poll_fd();
        let mut fds = [PollFd::new(&fd, interest | PollFlags::ERR | PollFlags::HUP)];
        poll(&mut fds, None).map_err(|error| BackendError::Io {
            message: error.to_string(),
        })?;
        Ok(())
    }

    fn flush_blocking(&self) -> BackendResult<()> {
        loop {
            match self.transport.flush_once().map_err(wayland_error)? {
                FlushOnce::Complete => return Ok(()),
                FlushOnce::Progress { .. } => {}
                FlushOnce::WouldBlock => self.wait(PollFlags::OUT)?,
            }
        }
    }
}

impl Globals {
    fn record(&mut self, name: u32, interface: String, version: u32) -> BackendResult<bool> {
        let slot = match interface.as_str() {
            "river_window_manager_v1" => &mut self.window_manager,
            "river_xkb_bindings_v1" => &mut self.xkb_bindings,
            "river_layer_shell_v1" => &mut self.layer_shell,
            "river_input_manager_v1" => &mut self.input_manager,
            "river_libinput_config_v1" => &mut self.libinput_config,
            _ => return Ok(false),
        };
        *slot = Some(AdvertisedGlobal { name, version });
        Ok(false)
    }

    fn remove(&mut self, name: u32) -> BackendResult<bool> {
        for slot in [
            &mut self.window_manager,
            &mut self.xkb_bindings,
            &mut self.layer_shell,
            &mut self.input_manager,
            &mut self.libinput_config,
        ] {
            if slot.as_ref().is_some_and(|global| global.name == name) {
                *slot = None;
            }
        }
        Ok(false)
    }

    fn validate(&self) -> BackendResult<()> {
        require_global("river_window_manager_v1", self.window_manager.as_ref(), 4)?;
        require_global("river_xkb_bindings_v1", self.xkb_bindings.as_ref(), 3)?;
        require_global("river_layer_shell_v1", self.layer_shell.as_ref(), 1)?;
        require_global("river_input_manager_v1", self.input_manager.as_ref(), 2)?;
        require_global("river_libinput_config_v1", self.libinput_config.as_ref(), 2)
    }
}

fn require_global(
    interface: &str,
    global: Option<&AdvertisedGlobal>,
    required: u32,
) -> BackendResult<()> {
    let advertised = global.map_or(0, |global| global.version);
    if advertised < required {
        return Err(BackendError::Unavailable {
            message: format!(
                "required global {interface} advertised version {advertised}, required {required}"
            ),
        });
    }
    Ok(())
}

fn bind_global<I: Proxy + 'static>(
    registry: &wl_registry::WlRegistry,
    global: &AdvertisedGlobal,
    version: u32,
    incoming: &SyncSender<Incoming>,
) -> BackendResult<I> {
    registry
        .send_constructor::<I>(
            wl_registry::Request::Bind {
                name: global.name,
                id: (I::interface(), version),
            },
            DirectData::new(ObjectKind::NoEvents, incoming),
        )
        .map_err(invalid_id)
}

fn full_capabilities() -> Capabilities {
    Capabilities {
        exact_geometry: true,
        server_side_borders: true,
        hide_show: true,
        explicit_ordering: true,
        fullscreen: true,
        unsupported: Vec::new(),
    }
}

fn invalid_id(error: wayland_client::backend::InvalidId) -> BackendError {
    BackendError::Io {
        message: format!("invalid Wayland object id: {error}"),
    }
}

fn io_error(error: io::Error) -> BackendError {
    BackendError::Io {
        message: error.to_string(),
    }
}

fn wayland_error(error: wayland_client::backend::WaylandError) -> BackendError {
    match error {
        wayland_client::backend::WaylandError::Io(error)
            if error.kind() == io::ErrorKind::UnexpectedEof
                || error.kind() == io::ErrorKind::ConnectionReset
                || error.kind() == io::ErrorKind::BrokenPipe =>
        {
            BackendError::Disconnected
        }
        wayland_client::backend::WaylandError::Io(error) => io_error(error),
        wayland_client::backend::WaylandError::Protocol(_) => protocol_error(),
    }
}

fn protocol_error() -> BackendError {
    BackendError::Protocol {
        kind: super::BackendProtocolErrorKind::InvalidFraming,
    }
}
