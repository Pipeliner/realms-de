//! Production River v0.4.8 window-management backend.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::Arc;
use std::time::Instant;

use realm_core::ipc::Capabilities;
use realm_core::WinId;
use rustix::event::{poll, PollFd, PollFlags};
use wayland_backend::client::{
    Backend, BoundedReadEventsGuard, DispatchOne, FlushOnce, ObjectData, ObjectId, ReadOnce,
};
use wayland_backend::protocol::Message;
use wayland_client::protocol::{wl_callback, wl_display, wl_registry};
use wayland_client::{Connection, Proxy, WEnum};

use super::river_protocols::{
    river_input_device_v1::{self, RiverInputDeviceV1},
    river_input_manager_v1::RiverInputManagerV1,
    river_layer_shell_output_v1::{self, RiverLayerShellOutputV1},
    river_layer_shell_seat_v1::{self, RiverLayerShellSeatV1},
    river_layer_shell_v1::{self, RiverLayerShellV1},
    river_libinput_config_v1::RiverLibinputConfigV1,
    river_libinput_device_v1::{self, RiverLibinputDeviceV1},
    river_libinput_result_v1::{self, RiverLibinputResultV1},
    river_node_v1::RiverNodeV1,
    river_output_v1::{self, RiverOutputV1},
    river_seat_v1::{self, RiverSeatV1},
    river_window_manager_v1::{self, RiverWindowManagerV1},
    river_window_v1::{self, RiverWindowV1},
    river_xkb_binding_v1::{self, RiverXkbBindingV1},
    river_xkb_bindings_seat_v1::{self, RiverXkbBindingsSeatV1},
    river_xkb_bindings_v1::{self, RiverXkbBindingsV1},
};
use super::{
    BackendBindingId, BackendBindingSpec, BackendCapacityResource, BackendContractError,
    BackendError, BackendEvent, BackendExitPolicy, BackendModifier, BackendNextKeyEdge,
    BackendPolicyEvent, BackendPolicyResponse, BackendPolicyTurn, BackendPolicyTurnId,
    BackendPollInterest, BackendReady, BackendResult, BackendSubmission, BackendTicket,
    BackendWindowId, WmBackend, KEY_REPEAT_DELAY_MS, KEY_REPEAT_RATE_HZ, MAX_BACKEND_INPUT_DEVICES,
    MAX_BACKEND_LIBINPUT_DEVICES, MAX_BACKEND_OUTPUTS, MAX_BACKEND_SEATS, MAX_CONFIGURED_BINDINGS,
    MAX_MANAGED_WINDOWS, MAX_POLICY_EVENTS, MAX_POLICY_TEXT_BYTES, MAX_REPLAY_POLICY_EVENTS,
};

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
    InputDeviceCreated(RiverInputDeviceV1),
    InputDeviceRemoved(RiverInputDeviceV1),
    InputDeviceType {
        device: RiverInputDeviceV1,
        keyboard: bool,
    },
    InputDeviceDone(RiverInputDeviceV1),
    LibinputDeviceCreated(RiverLibinputDeviceV1),
    LibinputDeviceRemoved(RiverLibinputDeviceV1),
    LibinputTapSupport {
        device: RiverLibinputDeviceV1,
        finger_count: i32,
    },
    LibinputTapCurrent {
        device: RiverLibinputDeviceV1,
        enabled: bool,
    },
    LibinputDeviceDone(RiverLibinputDeviceV1),
    LibinputResult {
        result: RiverLibinputResultV1,
        success: bool,
    },
    ManagerUnavailable,
    ManageStart,
    RenderStart,
    WindowCreated(RiverWindowV1),
    WindowClosed(RiverWindowV1),
    WindowDimensions {
        window: RiverWindowV1,
        width: i32,
        height: i32,
    },
    WindowAppId {
        window: RiverWindowV1,
        app_id: String,
    },
    WindowTitle {
        window: RiverWindowV1,
        title: String,
    },
    WindowIdentifier {
        window: RiverWindowV1,
        identifier: String,
    },
    OutputCreated(RiverOutputV1),
    OutputRemoved(RiverOutputV1),
    OutputPosition {
        output: RiverOutputV1,
        x: i32,
        y: i32,
    },
    OutputDimensions {
        output: RiverOutputV1,
        width: i32,
        height: i32,
    },
    SeatCreated(RiverSeatV1),
    SeatRemoved(RiverSeatV1),
    WindowInteracted(RiverWindowV1),
    Workarea {
        output: RiverLayerShellOutputV1,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    ExclusiveFocus(bool),
    BindingPressed(BackendBindingId),
    BindingReleased(BackendBindingId),
    BindingRepeatStopped(BackendBindingId),
    UnboundKeyEaten,
    ModifiersChanged {
        old: u32,
        new: u32,
    },
    Malformed,
}

#[derive(Debug, Clone, Copy)]
enum ObjectKind {
    Registry,
    Callback,
    InputManager,
    InputDevice,
    LibinputConfig,
    LibinputDevice,
    LibinputResult,
    WindowManager,
    Window,
    Output,
    Seat,
    LayerOutput,
    LayerSeat,
    XkbSeat,
    Binding(BackendBindingId),
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
            ObjectKind::InputManager => {
                match RiverInputManagerV1::parse_event(&connection, message) {
                    Ok((
                        _,
                        super::river_protocols::river_input_manager_v1::Event::InputDevice { id },
                    )) => {
                        self.emit(Incoming::InputDeviceCreated(id));
                        return Some(DirectData::new(ObjectKind::InputDevice, &self.incoming));
                    }
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::InputDevice => {
                match RiverInputDeviceV1::parse_event(&connection, message) {
                    Ok((device, river_input_device_v1::Event::Removed)) => {
                        self.emit(Incoming::InputDeviceRemoved(device));
                    }
                    Ok((device, river_input_device_v1::Event::Type { _type })) => match _type {
                        WEnum::Value(kind) => self.emit(Incoming::InputDeviceType {
                            device,
                            keyboard: kind == river_input_device_v1::Type::Keyboard,
                        }),
                        WEnum::Unknown(_) => self.emit(Incoming::Malformed),
                    },
                    Ok((device, river_input_device_v1::Event::Name { .. })) => drop(device),
                    Ok((device, river_input_device_v1::Event::Done)) => {
                        self.emit(Incoming::InputDeviceDone(device));
                    }
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::LibinputConfig => {
                match RiverLibinputConfigV1::parse_event(&connection, message) {
                    Ok((
                        _,
                        super::river_protocols::river_libinput_config_v1::Event::LibinputDevice {
                            id,
                        },
                    )) => {
                        self.emit(Incoming::LibinputDeviceCreated(id));
                        return Some(DirectData::new(ObjectKind::LibinputDevice, &self.incoming));
                    }
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::LibinputDevice => {
                match RiverLibinputDeviceV1::parse_event(&connection, message) {
                    Ok((device, river_libinput_device_v1::Event::Removed)) => {
                        self.emit(Incoming::LibinputDeviceRemoved(device));
                    }
                    Ok((device, river_libinput_device_v1::Event::TapSupport { finger_count })) => {
                        self.emit(Incoming::LibinputTapSupport {
                            device,
                            finger_count,
                        });
                    }
                    Ok((device, river_libinput_device_v1::Event::TapCurrent { state })) => {
                        match state {
                            WEnum::Value(state) => self.emit(Incoming::LibinputTapCurrent {
                                device,
                                enabled: state == river_libinput_device_v1::TapState::Enabled,
                            }),
                            WEnum::Unknown(_) => self.emit(Incoming::Malformed),
                        }
                    }
                    Ok((device, river_libinput_device_v1::Event::Done)) => {
                        self.emit(Incoming::LibinputDeviceDone(device));
                    }
                    Ok(_) => {}
                    Err(_) => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::LibinputResult => {
                match RiverLibinputResultV1::parse_event(&connection, message) {
                    Ok((result, river_libinput_result_v1::Event::Success)) => {
                        self.emit(Incoming::LibinputResult {
                            result,
                            success: true,
                        });
                    }
                    Ok((result, river_libinput_result_v1::Event::Unsupported))
                    | Ok((result, river_libinput_result_v1::Event::Invalid)) => {
                        self.emit(Incoming::LibinputResult {
                            result,
                            success: false,
                        });
                    }
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::WindowManager => {
                match RiverWindowManagerV1::parse_event(&connection, message) {
                    Ok((_, river_window_manager_v1::Event::Unavailable)) => {
                        self.emit(Incoming::ManagerUnavailable);
                    }
                    Ok((_, river_window_manager_v1::Event::ManageStart)) => {
                        self.emit(Incoming::ManageStart);
                    }
                    Ok((_, river_window_manager_v1::Event::RenderStart)) => {
                        self.emit(Incoming::RenderStart);
                    }
                    Ok((_, river_window_manager_v1::Event::Window { id })) => {
                        self.emit(Incoming::WindowCreated(id));
                        return Some(DirectData::new(ObjectKind::Window, &self.incoming));
                    }
                    Ok((_, river_window_manager_v1::Event::Output { id })) => {
                        self.emit(Incoming::OutputCreated(id));
                        return Some(DirectData::new(ObjectKind::Output, &self.incoming));
                    }
                    Ok((_, river_window_manager_v1::Event::Seat { id })) => {
                        self.emit(Incoming::SeatCreated(id));
                        return Some(DirectData::new(ObjectKind::Seat, &self.incoming));
                    }
                    Ok((_, river_window_manager_v1::Event::SessionLocked))
                    | Ok((_, river_window_manager_v1::Event::SessionUnlocked)) => {}
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::Window => match RiverWindowV1::parse_event(&connection, message) {
                Ok((window, river_window_v1::Event::Closed)) => {
                    self.emit(Incoming::WindowClosed(window));
                }
                Ok((window, river_window_v1::Event::Dimensions { width, height })) => {
                    self.emit(Incoming::WindowDimensions {
                        window,
                        width,
                        height,
                    });
                }
                Ok((window, river_window_v1::Event::AppId { app_id })) => {
                    self.emit(Incoming::WindowAppId {
                        window,
                        app_id: app_id.unwrap_or_default(),
                    });
                }
                Ok((window, river_window_v1::Event::Title { title })) => {
                    self.emit(Incoming::WindowTitle {
                        window,
                        title: title.unwrap_or_default(),
                    });
                }
                Ok((window, river_window_v1::Event::Identifier { identifier })) => {
                    self.emit(Incoming::WindowIdentifier { window, identifier });
                }
                Ok(_) => {}
                Err(_) => self.emit(Incoming::Malformed),
            },
            ObjectKind::Output => match RiverOutputV1::parse_event(&connection, message) {
                Ok((output, river_output_v1::Event::Removed)) => {
                    self.emit(Incoming::OutputRemoved(output));
                }
                Ok((output, river_output_v1::Event::Position { x, y })) => {
                    self.emit(Incoming::OutputPosition { output, x, y });
                }
                Ok((output, river_output_v1::Event::Dimensions { width, height })) => {
                    self.emit(Incoming::OutputDimensions {
                        output,
                        width,
                        height,
                    });
                }
                Ok(_) => {}
                Err(_) => self.emit(Incoming::Malformed),
            },
            ObjectKind::Seat => match RiverSeatV1::parse_event(&connection, message) {
                Ok((seat, river_seat_v1::Event::Removed)) => {
                    self.emit(Incoming::SeatRemoved(seat));
                }
                Ok((_, river_seat_v1::Event::WindowInteraction { window })) => {
                    self.emit(Incoming::WindowInteracted(window));
                }
                Ok(_) => {}
                Err(_) => self.emit(Incoming::Malformed),
            },
            ObjectKind::LayerOutput => {
                match RiverLayerShellOutputV1::parse_event(&connection, message) {
                    Ok((
                        output,
                        river_layer_shell_output_v1::Event::NonExclusiveArea {
                            x,
                            y,
                            width,
                            height,
                        },
                    )) => self.emit(Incoming::Workarea {
                        output,
                        x,
                        y,
                        width,
                        height,
                    }),
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::LayerSeat => {
                match RiverLayerShellSeatV1::parse_event(&connection, message) {
                    Ok((_, river_layer_shell_seat_v1::Event::FocusExclusive)) => {
                        self.emit(Incoming::ExclusiveFocus(true));
                    }
                    Ok((_, river_layer_shell_seat_v1::Event::FocusNonExclusive))
                    | Ok((_, river_layer_shell_seat_v1::Event::FocusNone)) => {
                        self.emit(Incoming::ExclusiveFocus(false));
                    }
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::XkbSeat => {
                match RiverXkbBindingsSeatV1::parse_event(&connection, message) {
                    Ok((_, river_xkb_bindings_seat_v1::Event::AteUnboundKey)) => {
                        self.emit(Incoming::UnboundKeyEaten);
                    }
                    Ok((_, river_xkb_bindings_seat_v1::Event::ModifiersUpdate { old, new })) => {
                        self.emit(Incoming::ModifiersChanged {
                            old: old.into(),
                            new: new.into(),
                        });
                    }
                    _ => self.emit(Incoming::Malformed),
                }
            }
            ObjectKind::Binding(id) => match RiverXkbBindingV1::parse_event(&connection, message) {
                Ok((_, river_xkb_binding_v1::Event::Pressed)) => {
                    self.emit(Incoming::BindingPressed(id));
                }
                Ok((_, river_xkb_binding_v1::Event::Released)) => {
                    self.emit(Incoming::BindingReleased(id));
                }
                Ok((_, river_xkb_binding_v1::Event::StopRepeat)) => {
                    self.emit(Incoming::BindingRepeatStopped(id));
                }
                _ => self.emit(Incoming::Malformed),
            },
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

#[derive(Debug)]
struct InputDevice {
    proxy: RiverInputDeviceV1,
    keyboard: Option<bool>,
}

#[derive(Debug)]
struct LibinputDevice {
    proxy: RiverLibinputDeviceV1,
    tap_support: Option<i32>,
    tap_enabled: Option<bool>,
}

#[derive(Debug)]
struct Window {
    proxy: RiverWindowV1,
    node: RiverNodeV1,
    ordinal: u64,
    backend_id: Option<BackendWindowId>,
    app_id: String,
    title: String,
    pending_open: bool,
    assigned: Option<realm_core::WinId>,
    dimensions: Option<(i32, i32)>,
    proposed_size: Option<(i32, i32)>,
    awaiting_dimensions: bool,
    position: Option<(i32, i32)>,
    focused: Option<bool>,
    hidden: Option<bool>,
    fullscreen: bool,
    initialized: bool,
}

#[derive(Debug)]
struct Output {
    proxy: RiverOutputV1,
    layer: RiverLayerShellOutputV1,
    ordinal: u64,
    position: Option<(i32, i32)>,
    dimensions: Option<(i32, i32)>,
    workarea: Option<realm_core::layout::Rect>,
}

#[derive(Debug)]
struct Seat {
    proxy: RiverSeatV1,
    ordinal: u64,
}

#[derive(Debug)]
struct Binding {
    proxy: RiverXkbBindingV1,
    enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseStage {
    AwaitRender,
    AwaitCorrectionManage,
    FlushingRender,
}

#[derive(Debug)]
struct PendingResponse {
    ticket: BackendTicket,
    response: BackendPolicyResponse,
    stage: ResponseStage,
    terminal_error: Option<BackendError>,
    corrections: HashMap<ObjectId, ((i32, i32), (i32, i32))>,
    correction_round: bool,
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
    input_devices: HashMap<ObjectId, InputDevice>,
    libinput_devices: HashMap<ObjectId, LibinputDevice>,
    libinput_results: HashSet<ObjectId>,
    next_input_ordinal: u64,
    next_libinput_ordinal: u64,
    windows: HashMap<ObjectId, Window>,
    window_ids: HashMap<BackendWindowId, ObjectId>,
    realm_ids: BTreeMap<realm_core::WinId, BackendWindowId>,
    outputs: HashMap<ObjectId, Output>,
    seats: HashMap<ObjectId, Seat>,
    selected_output: Option<ObjectId>,
    selected_seat: Option<ObjectId>,
    layer_seat: Option<RiverLayerShellSeatV1>,
    xkb_seat: Option<RiverXkbBindingsSeatV1>,
    next_window_ordinal: u64,
    next_output_ordinal: u64,
    next_seat_ordinal: u64,
    replay: bool,
    policy_events: Vec<BackendPolicyEvent>,
    public_events: VecDeque<BackendEvent>,
    open_turn: Option<BackendPolicyTurnId>,
    turn_requested: bool,
    pending_response: Option<PendingResponse>,
    next_turn: u64,
    bindings: BTreeMap<super::BackendBindingId, BackendBindingSpec>,
    binding_objects: BTreeMap<BackendBindingId, Binding>,
    watched_modifiers: Vec<BackendModifier>,
    default_output: Option<ObjectId>,
    focused_window: Option<WinId>,
    exclusive_focus: bool,
    drain_ticket: Option<BackendTicket>,
    selected_output_lost: bool,
    terminal_after_response: Option<BackendError>,
    last_projection: Option<Vec<realm_core::layout::Placement>>,
    deferred_corrections: HashMap<ObjectId, ((i32, i32), (i32, i32))>,
    read_guard: Option<BoundedReadEventsGuard>,
    flush_pending: bool,
    flush_blocked: bool,
    exit_started: bool,
    exit_flushed: bool,
    exit_complete: bool,
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
            input_devices: HashMap::new(),
            libinput_devices: HashMap::new(),
            libinput_results: HashSet::new(),
            next_input_ordinal: 0,
            next_libinput_ordinal: 0,
            windows: HashMap::new(),
            window_ids: HashMap::new(),
            realm_ids: BTreeMap::new(),
            outputs: HashMap::new(),
            seats: HashMap::new(),
            selected_output: None,
            selected_seat: None,
            layer_seat: None,
            xkb_seat: None,
            next_window_ordinal: 0,
            next_output_ordinal: 0,
            next_seat_ordinal: 0,
            replay: true,
            policy_events: Vec::new(),
            public_events: VecDeque::new(),
            open_turn: None,
            turn_requested: false,
            pending_response: None,
            next_turn: 0,
            bindings: BTreeMap::new(),
            binding_objects: BTreeMap::new(),
            watched_modifiers: Vec::new(),
            default_output: None,
            focused_window: None,
            exclusive_focus: false,
            drain_ticket: None,
            selected_output_lost: false,
            terminal_after_response: None,
            last_projection: None,
            deferred_corrections: HashMap::new(),
            read_guard: None,
            flush_pending: false,
            flush_blocked: false,
            exit_started: false,
            exit_flushed: false,
            exit_complete: false,
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
            _ => Err(protocol_error()),
        })?;
        globals.validate()?;

        self.layer_shell = Some(bind_global::<RiverLayerShellV1>(
            &registry,
            globals.layer_shell.as_ref().unwrap(),
            1,
            &self.incoming_tx,
            ObjectKind::NoEvents,
        )?);
        self.xkb_bindings = Some(bind_global::<RiverXkbBindingsV1>(
            &registry,
            globals.xkb_bindings.as_ref().unwrap(),
            3,
            &self.incoming_tx,
            ObjectKind::NoEvents,
        )?);
        self.input_manager = Some(bind_global::<RiverInputManagerV1>(
            &registry,
            globals.input_manager.as_ref().unwrap(),
            2,
            &self.incoming_tx,
            ObjectKind::InputManager,
        )?);
        self.libinput_config = Some(bind_global::<RiverLibinputConfigV1>(
            &registry,
            globals.libinput_config.as_ref().unwrap(),
            2,
            &self.incoming_tx,
            ObjectKind::LibinputConfig,
        )?);

        let input_discovery = self.sync()?;
        let mut input_events = Vec::new();
        self.pump_until_sync(input_discovery, |incoming| match incoming {
            Incoming::SyncDone => Ok(true),
            Incoming::Global { .. } | Incoming::GlobalRemoved(_) => Ok(false),
            Incoming::Malformed => Err(protocol_error()),
            incoming => {
                input_events.push(incoming);
                Ok(false)
            }
        })?;
        for event in input_events {
            self.apply_input_event(event)?;
        }
        let input_results = self.sync()?;
        let mut result_events = Vec::new();
        self.pump_until_sync(input_results, |incoming| match incoming {
            Incoming::SyncDone => Ok(true),
            Incoming::Global { .. } | Incoming::GlobalRemoved(_) => Ok(false),
            Incoming::Malformed => Err(protocol_error()),
            incoming => {
                result_events.push(incoming);
                Ok(false)
            }
        })?;
        for event in result_events {
            self.apply_input_event(event)?;
        }
        if !self.libinput_results.is_empty() {
            return Err(protocol_error());
        }

        self.window_manager = Some(bind_global::<RiverWindowManagerV1>(
            &registry,
            globals.window_manager.as_ref().unwrap(),
            globals.window_manager.as_ref().unwrap().version.min(5),
            &self.incoming_tx,
            ObjectKind::WindowManager,
        )?);
        self.flush_blocking()?;
        self.connected = true;
        Ok(full_capabilities())
    }

    /// Register the stable, bounded binding mechanism catalogue.
    pub fn configure_bindings(&mut self, bindings: Vec<BackendBindingSpec>) -> BackendResult<()> {
        if self.selected_seat.is_some() || !self.binding_objects.is_empty() {
            return Err(protocol_error());
        }
        if bindings.len() > MAX_CONFIGURED_BINDINGS {
            return Err(capacity(
                BackendCapacityResource::ConfiguredBindings,
                MAX_CONFIGURED_BINDINGS as u64,
            ));
        }
        let mut configured = BTreeMap::new();
        for binding in bindings {
            if keysym(&binding.keysym).is_none()
                || !binding.modifiers.windows(2).all(|pair| pair[0] < pair[1])
                || configured.insert(binding.id, binding).is_some()
            {
                return Err(protocol_error());
            }
        }
        self.bindings = configured;
        Ok(())
    }

    /// Bind a live River window identity to the Realm id allocated by Session.
    pub fn assign_window(
        &mut self,
        backend_id: &BackendWindowId,
        win: WinId,
    ) -> Result<(), BackendContractError> {
        let object_id = self.window_ids.get(backend_id).cloned().ok_or_else(|| {
            BackendContractError::UnknownWindow {
                backend_id: backend_id.clone(),
            }
        })?;
        let window = self.windows.get_mut(&object_id).ok_or_else(|| {
            BackendContractError::UnknownWindow {
                backend_id: backend_id.clone(),
            }
        })?;
        if let Some(assigned) = window.assigned {
            return if assigned == win {
                Ok(())
            } else {
                Err(BackendContractError::ConflictingBackendIdentity)
            };
        }
        if self.realm_ids.contains_key(&win) {
            return Err(BackendContractError::ConflictingRealmIdentity);
        }
        window.assigned = Some(win);
        self.realm_ids.insert(win, backend_id.clone());
        Ok(())
    }

    /// Submit the complete answer to River's current manage phase.
    pub fn respond_policy_turn(
        &mut self,
        turn: BackendPolicyTurnId,
        ticket: BackendTicket,
        response: BackendPolicyResponse,
    ) -> BackendResult<BackendSubmission> {
        if self.open_turn != Some(turn) || self.pending_response.is_some() {
            return Err(protocol_error());
        }
        let projection_len = response.projection.as_ref().map_or(0, Vec::len);
        if response.closes.len().saturating_add(projection_len) > super::MAX_STAGED_EFFECTS {
            return Err(capacity(
                BackendCapacityResource::PolicyEffects,
                super::MAX_STAGED_EFFECTS as u64,
            ));
        }
        let mut placed = HashSet::new();
        let mut closed = HashSet::new();
        for win in response.closes.iter().chain(
            response
                .projection
                .iter()
                .flat_map(|projection| projection.iter().map(|placement| &placement.win)),
        ) {
            let backend_id = self.realm_ids.get(win).ok_or_else(protocol_error)?;
            if !self.window_ids.contains_key(backend_id) {
                return Err(protocol_error());
            }
        }
        if response.closes.iter().any(|win| !closed.insert(*win)) {
            return Err(protocol_error());
        }
        if response.projection.as_ref().is_some_and(|projection| {
            projection.iter().any(|placement| {
                placement.rect.w <= 0 || placement.rect.h <= 0 || !placed.insert(placement.win)
            })
        }) || !response
            .bindings
            .enabled
            .windows(2)
            .all(|pair| pair[0] < pair[1])
            || !response
                .bindings
                .watched_modifiers
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            || self.terminal_after_response.is_none()
                && response
                    .bindings
                    .enabled
                    .iter()
                    .any(|id| !self.binding_objects.contains_key(id))
        {
            return Err(protocol_error());
        }
        let terminal_error = self.terminal_after_response.take();
        let mut correction_round = false;
        if terminal_error.is_none() {
            for win in &response.closes {
                let backend_id = self.realm_ids.get(win).ok_or_else(protocol_error)?;
                let object_id = self.window_ids.get(backend_id).ok_or_else(protocol_error)?;
                self.windows
                    .get(object_id)
                    .ok_or_else(protocol_error)?
                    .proxy
                    .close();
            }
            self.apply_manage_response(&response)?;
            let requested_sizes: HashMap<_, _> = response
                .projection
                .as_ref()
                .map(|projection| {
                    projection
                        .iter()
                        .filter_map(|placement| {
                            let backend_id = self.realm_ids.get(&placement.win)?;
                            let object_id = self.window_ids.get(backend_id)?.clone();
                            Some((object_id, (placement.rect.w, placement.rect.h)))
                        })
                        .collect()
                })
                .unwrap_or_default();
            for (id, (target, corrected)) in std::mem::take(&mut self.deferred_corrections) {
                if !self.windows.contains_key(&id) {
                    continue;
                }
                if response.projection.is_some()
                    && requested_sizes.get(&id).copied() != Some(target)
                {
                    continue;
                }
                let window = self.windows.get_mut(&id).ok_or_else(protocol_error)?;
                window.proxy.propose_dimensions(corrected.0, corrected.1);
                window.proposed_size = Some(corrected);
                window.awaiting_dimensions = true;
                correction_round = true;
            }
        }
        self.window_manager
            .as_ref()
            .ok_or_else(protocol_error)?
            .manage_finish();
        self.open_turn = None;
        self.pending_response = Some(PendingResponse {
            ticket,
            response,
            stage: ResponseStage::AwaitRender,
            terminal_error,
            corrections: HashMap::new(),
            correction_round,
        });
        self.flush_pending = true;
        Ok(BackendSubmission::Pending)
    }

    fn apply_manage_response(&mut self, response: &BackendPolicyResponse) -> BackendResult<()> {
        let Some(output_id) = self.selected_output.clone() else {
            if response.projection.is_none()
                && response.bindings.enabled.is_empty()
                && response.bindings.watched_modifiers.is_empty()
                && response.bindings.next_key_edge == BackendNextKeyEdge::Preserve
            {
                return Ok(());
            }
            return Err(protocol_error());
        };
        let output = self.outputs.get(&output_id).ok_or_else(protocol_error)?;
        let (ox, oy) = output.position.ok_or_else(protocol_error)?;
        let (ow, oh) = output.dimensions.ok_or_else(protocol_error)?;
        let output_rect = realm_core::layout::Rect::new(ox, oy, ow, oh);
        let output_proxy = output.proxy.clone();
        if self.default_output.as_ref() != Some(&output_id) {
            output.layer.set_default();
            self.default_output = Some(output_id);
        }

        if let Some(projection) = &response.projection {
            let fullscreen = projection.len() == 1 && projection[0].rect == output_rect;
            for placement in projection {
                let backend_id = self
                    .realm_ids
                    .get(&placement.win)
                    .ok_or_else(protocol_error)?;
                let object_id = self.window_ids.get(backend_id).ok_or_else(protocol_error)?;
                let window = self.windows.get_mut(object_id).ok_or_else(protocol_error)?;
                if !window.initialized {
                    window
                        .proxy
                        .set_tiled(river_window_v1::Edges::from_bits_truncate(15));
                    window
                        .proxy
                        .set_capabilities(river_window_v1::Capabilities::from_bits_truncate(4));
                    window.initialized = true;
                }
                if fullscreen {
                    if !window.fullscreen {
                        window.proxy.fullscreen(&output_proxy);
                        window.proxy.inform_fullscreen();
                        window.fullscreen = true;
                        window.awaiting_dimensions = false;
                    }
                } else {
                    if window.fullscreen {
                        window.proxy.exit_fullscreen();
                        window.proxy.inform_not_fullscreen();
                        window.fullscreen = false;
                        window.proposed_size = None;
                        window.awaiting_dimensions = false;
                        window.position = None;
                    }
                    let size = (placement.rect.w, placement.rect.h);
                    if window.proposed_size != Some(size) {
                        window.proxy.propose_dimensions(size.0, size.1);
                        window.proposed_size = Some(size);
                        window.awaiting_dimensions = true;
                    }
                }
            }
            let focused = projection
                .iter()
                .find(|placement| placement.focused)
                .map(|placement| placement.win);
            if !self.exclusive_focus && self.focused_window != focused {
                match focused {
                    Some(win) => {
                        let backend_id = self.realm_ids.get(&win).ok_or_else(protocol_error)?;
                        let object_id =
                            self.window_ids.get(backend_id).ok_or_else(protocol_error)?;
                        let window = self.windows.get(object_id).ok_or_else(protocol_error)?;
                        self.selected_seat_proxy()?.focus_window(&window.proxy);
                    }
                    None => self.selected_seat_proxy()?.clear_focus(),
                }
                self.focused_window = focused;
            }
        }

        for (id, binding) in &mut self.binding_objects {
            let enabled = response.bindings.enabled.binary_search(id).is_ok();
            if binding.enabled != enabled {
                if enabled {
                    binding.proxy.enable();
                } else {
                    binding.proxy.disable();
                }
                binding.enabled = enabled;
            }
        }
        if self.watched_modifiers != response.bindings.watched_modifiers {
            self.xkb_seat
                .as_ref()
                .ok_or_else(protocol_error)?
                .modifiers_watch(river_seat_v1::Modifiers::from_bits_truncate(
                    modifiers_bits(&response.bindings.watched_modifiers),
                ));
            self.watched_modifiers = response.bindings.watched_modifiers.clone();
        }
        match response.bindings.next_key_edge {
            BackendNextKeyEdge::Preserve => {}
            BackendNextKeyEdge::Ensure => self
                .xkb_seat
                .as_ref()
                .ok_or_else(protocol_error)?
                .ensure_next_key_eaten(),
            BackendNextKeyEdge::Cancel => self
                .xkb_seat
                .as_ref()
                .ok_or_else(protocol_error)?
                .cancel_ensure_next_key_eaten(),
        }
        Ok(())
    }

    fn selected_seat_proxy(&self) -> BackendResult<&RiverSeatV1> {
        let id = self.selected_seat.as_ref().ok_or_else(protocol_error)?;
        self.seats
            .get(id)
            .map(|seat| &seat.proxy)
            .ok_or_else(protocol_error)
    }

    /// Return the bounded transport interest for the daemon poll set.
    pub fn poll_interest(&self) -> BackendPollInterest {
        BackendPollInterest {
            immediate: self.flush_pending && !self.flush_blocked
                || !self.public_events.is_empty()
                || self.open_turn.is_none() && self.read_guard.is_none(),
            readable: self.connected && !self.exit_started,
            writable: self.flush_pending && self.flush_blocked,
        }
    }

    /// Perform one bounded River transport or public-dequeue quantum.
    pub fn service(
        &mut self,
        ready: BackendReady,
        _now: Instant,
    ) -> BackendResult<Option<BackendEvent>> {
        if self.exit_started && ready.terminal && !self.exit_flushed {
            return Err(BackendError::Disconnected);
        }
        if self.flush_pending && (!self.flush_blocked || ready.writable) {
            match self.transport.flush_once().map_err(wayland_error)? {
                FlushOnce::Complete => {
                    self.flush_pending = false;
                    self.flush_blocked = false;
                    if self.exit_started {
                        self.exit_flushed = true;
                        self.pending_response = None;
                    } else if self
                        .pending_response
                        .as_ref()
                        .is_some_and(|pending| pending.stage == ResponseStage::FlushingRender)
                    {
                        let pending = self.pending_response.take().expect("stage checked above");
                        let result = pending.terminal_error.map_or(Ok(()), Err);
                        let successful = result.is_ok();
                        if successful {
                            if let Some(projection) = pending.response.projection {
                                self.last_projection = Some(projection);
                            }
                        }
                        self.public_events
                            .push_back(BackendEvent::OperationCompleted {
                                ticket: pending.ticket,
                                result,
                            });
                        if successful {
                            self.drain_ticket = Some(pending.ticket);
                        }
                    }
                }
                FlushOnce::Progress { .. } => self.flush_blocked = false,
                FlushOnce::WouldBlock => self.flush_blocked = true,
            }
            return Ok(None);
        }
        if self.exit_started {
            if self.exit_complete {
                return Err(protocol_error());
            }
            if ready.terminal && self.exit_flushed {
                self.exit_complete = true;
                return Ok(Some(BackendEvent::Disconnected));
            }
            return Ok(None);
        }
        if let Some(event) = self.public_events.pop_front() {
            return Ok(Some(event));
        }
        if self.open_turn.is_some() {
            return Ok(None);
        }
        if self.read_guard.is_none() {
            match self
                .transport
                .dispatch_one_pending()
                .map_err(wayland_error)?
            {
                DispatchOne::Dispatched => {
                    if let Ok(incoming) = self.incoming_rx.try_recv() {
                        self.apply_runtime_event(incoming)?;
                    }
                    return Ok(None);
                }
                DispatchOne::PreparedReadLive => return Err(protocol_error()),
                DispatchOne::NeedRead => {
                    if let Some(ticket) = self.drain_ticket.take() {
                        self.public_events
                            .push_back(BackendEvent::RetainedObservationsDrained { ticket });
                        return Ok(None);
                    }
                }
            }
        }
        if (ready.readable || ready.terminal) && self.read_guard.is_some() {
            let guard = self.read_guard.take().expect("guard checked above");
            match guard.read_once().map_err(wayland_error)? {
                ReadOnce::Read { .. } | ReadOnce::WouldBlock => return Ok(None),
            }
        }
        if self.read_guard.is_none() {
            self.read_guard = self.transport.prepare_read_bounded();
            return Ok(None);
        }
        Ok(None)
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

    fn apply_input_event(&mut self, event: Incoming) -> BackendResult<()> {
        match event {
            Incoming::InputDeviceCreated(proxy) => {
                if self.input_devices.len() == MAX_BACKEND_INPUT_DEVICES {
                    proxy.destroy();
                    return Err(capacity(
                        BackendCapacityResource::InputDevices,
                        MAX_BACKEND_INPUT_DEVICES as u64,
                    ));
                }
                self.next_input_ordinal = self
                    .next_input_ordinal
                    .checked_add(1)
                    .ok_or_else(|| capacity(BackendCapacityResource::ObjectOrdinals, u64::MAX))?;
                self.input_devices.insert(
                    proxy.id(),
                    InputDevice {
                        proxy,
                        keyboard: None,
                    },
                );
            }
            Incoming::InputDeviceRemoved(proxy) => {
                self.input_devices.remove(&proxy.id());
                proxy.destroy();
                self.flush_pending |= self.connected;
            }
            Incoming::InputDeviceType { device, keyboard } => {
                self.input_devices
                    .get_mut(&device.id())
                    .ok_or_else(protocol_error)?
                    .keyboard = Some(keyboard);
            }
            Incoming::InputDeviceDone(proxy) => {
                let device = self
                    .input_devices
                    .get(&proxy.id())
                    .ok_or_else(protocol_error)?;
                if device.keyboard == Some(true) {
                    device
                        .proxy
                        .set_repeat_info(KEY_REPEAT_RATE_HZ as i32, KEY_REPEAT_DELAY_MS as i32);
                    self.flush_pending |= self.connected;
                }
            }
            Incoming::LibinputDeviceCreated(proxy) => {
                if self.libinput_devices.len() == MAX_BACKEND_LIBINPUT_DEVICES {
                    proxy.destroy();
                    return Err(capacity(
                        BackendCapacityResource::LibinputDevices,
                        MAX_BACKEND_LIBINPUT_DEVICES as u64,
                    ));
                }
                self.next_libinput_ordinal = self
                    .next_libinput_ordinal
                    .checked_add(1)
                    .ok_or_else(|| capacity(BackendCapacityResource::ObjectOrdinals, u64::MAX))?;
                self.libinput_devices.insert(
                    proxy.id(),
                    LibinputDevice {
                        proxy,
                        tap_support: None,
                        tap_enabled: None,
                    },
                );
            }
            Incoming::LibinputDeviceRemoved(proxy) => {
                self.libinput_devices.remove(&proxy.id());
                proxy.destroy();
                self.flush_pending |= self.connected;
            }
            Incoming::LibinputTapSupport {
                device,
                finger_count,
            } => {
                self.libinput_devices
                    .get_mut(&device.id())
                    .ok_or_else(protocol_error)?
                    .tap_support = Some(finger_count);
            }
            Incoming::LibinputTapCurrent { device, enabled } => {
                self.libinput_devices
                    .get_mut(&device.id())
                    .ok_or_else(protocol_error)?
                    .tap_enabled = Some(enabled);
            }
            Incoming::LibinputDeviceDone(proxy) => {
                let device = self
                    .libinput_devices
                    .get(&proxy.id())
                    .ok_or_else(protocol_error)?;
                if device.tap_support.is_some_and(|count| count > 0)
                    && device.tap_enabled == Some(false)
                {
                    let result = device
                        .proxy
                        .send_constructor::<RiverLibinputResultV1>(
                            river_libinput_device_v1::Request::SetTap {
                                state: WEnum::Value(river_libinput_device_v1::TapState::Enabled),
                            },
                            DirectData::new(ObjectKind::LibinputResult, &self.incoming_tx),
                        )
                        .map_err(invalid_id)?;
                    self.libinput_results.insert(result.id());
                    self.flush_pending |= self.connected;
                }
            }
            Incoming::LibinputResult { result, success } => {
                if !self.libinput_results.remove(&result.id()) || !success {
                    return Err(BackendError::Unavailable {
                        message: "River rejected required tap-to-click policy".to_owned(),
                    });
                }
            }
            _ => return Err(protocol_error()),
        }
        Ok(())
    }

    fn apply_runtime_event(&mut self, event: Incoming) -> BackendResult<()> {
        match event {
            Incoming::ManagerUnavailable => {
                return Err(BackendError::Unavailable {
                    message: "another River window manager is active".to_owned(),
                });
            }
            Incoming::WindowCreated(proxy) => {
                if self.windows.len() == MAX_MANAGED_WINDOWS {
                    proxy.destroy();
                    return Err(capacity(
                        BackendCapacityResource::ManagedWindows,
                        MAX_MANAGED_WINDOWS as u64,
                    ));
                }
                self.next_window_ordinal = self
                    .next_window_ordinal
                    .checked_add(1)
                    .ok_or_else(|| capacity(BackendCapacityResource::ObjectOrdinals, u64::MAX))?;
                let node = proxy
                    .send_constructor::<RiverNodeV1>(
                        river_window_v1::Request::GetNode {},
                        DirectData::new(ObjectKind::NoEvents, &self.incoming_tx),
                    )
                    .map_err(invalid_id)?;
                self.flush_pending = true;
                self.windows.insert(
                    proxy.id(),
                    Window {
                        proxy,
                        node,
                        ordinal: self.next_window_ordinal,
                        backend_id: None,
                        app_id: String::new(),
                        title: String::new(),
                        pending_open: true,
                        assigned: None,
                        dimensions: None,
                        proposed_size: None,
                        awaiting_dimensions: false,
                        position: None,
                        focused: None,
                        hidden: None,
                        fullscreen: false,
                        initialized: false,
                    },
                );
            }
            Incoming::WindowIdentifier { window, identifier } => {
                let backend_id = BackendWindowId::new(identifier).ok_or_else(protocol_error)?;
                if self.window_ids.contains_key(&backend_id) {
                    return Err(protocol_error());
                }
                self.windows
                    .get_mut(&window.id())
                    .ok_or_else(protocol_error)?
                    .backend_id = Some(backend_id.clone());
                self.window_ids.insert(backend_id, window.id());
            }
            Incoming::WindowAppId { window, app_id } => {
                self.windows
                    .get_mut(&window.id())
                    .ok_or_else(protocol_error)?
                    .app_id = app_id;
            }
            Incoming::WindowTitle { window, title } => {
                let record = self
                    .windows
                    .get_mut(&window.id())
                    .ok_or_else(protocol_error)?;
                record.title = title.clone();
                if !record.pending_open {
                    self.policy_events.push(BackendPolicyEvent::TitleChanged {
                        backend_id: record.backend_id.clone().ok_or_else(protocol_error)?,
                        title,
                    });
                }
            }
            Incoming::WindowDimensions {
                window,
                width,
                height,
            } => {
                if width <= 0 || height <= 0 {
                    return Err(protocol_error());
                }
                let record = self
                    .windows
                    .get_mut(&window.id())
                    .ok_or_else(protocol_error)?;
                let changed = record.dimensions != Some((width, height));
                let was_awaiting = record.awaiting_dimensions;
                record.awaiting_dimensions = false;
                record.dimensions = Some((width, height));
                if changed
                    && !was_awaiting
                    && !record.fullscreen
                    && self.pending_response.is_none()
                    && !record.pending_open
                {
                    if let (Some(backend_id), Some((x, y))) =
                        (record.backend_id.clone(), record.position)
                    {
                        self.policy_events
                            .push(BackendPolicyEvent::GeometryDrifted {
                                backend_id,
                                rect: realm_core::layout::Rect::new(x, y, width, height),
                            });
                        record.proposed_size = None;
                    }
                }
            }
            Incoming::WindowClosed(proxy) => {
                let Some(record) = self.windows.remove(&proxy.id()) else {
                    return Err(protocol_error());
                };
                record.node.destroy();
                if let Some(backend_id) = record.backend_id {
                    self.window_ids.remove(&backend_id);
                    if let Some(win) = record.assigned {
                        self.realm_ids.remove(&win);
                        if self.focused_window == Some(win) {
                            self.focused_window = None;
                        }
                    }
                    if !record.pending_open {
                        self.policy_events
                            .push(BackendPolicyEvent::WindowClosed(backend_id));
                    }
                }
                proxy.destroy();
                self.flush_pending = true;
            }
            Incoming::OutputCreated(proxy) => {
                if self.outputs.len() == MAX_BACKEND_OUTPUTS {
                    proxy.destroy();
                    return Err(capacity(
                        BackendCapacityResource::Outputs,
                        MAX_BACKEND_OUTPUTS as u64,
                    ));
                }
                self.next_output_ordinal = self
                    .next_output_ordinal
                    .checked_add(1)
                    .ok_or_else(|| capacity(BackendCapacityResource::ObjectOrdinals, u64::MAX))?;
                let layer = self
                    .layer_shell
                    .as_ref()
                    .ok_or_else(protocol_error)?
                    .send_constructor::<RiverLayerShellOutputV1>(
                        river_layer_shell_v1::Request::GetOutput {
                            output: proxy.clone(),
                        },
                        DirectData::new(ObjectKind::LayerOutput, &self.incoming_tx),
                    )
                    .map_err(invalid_id)?;
                self.flush_pending = true;
                self.outputs.insert(
                    proxy.id(),
                    Output {
                        proxy,
                        layer,
                        ordinal: self.next_output_ordinal,
                        position: None,
                        dimensions: None,
                        workarea: None,
                    },
                );
            }
            Incoming::OutputPosition { output, x, y } => {
                self.outputs
                    .get_mut(&output.id())
                    .ok_or_else(protocol_error)?
                    .position = Some((x, y));
            }
            Incoming::OutputDimensions {
                output,
                width,
                height,
            } => {
                if width <= 0 || height <= 0 {
                    return Err(protocol_error());
                }
                self.outputs
                    .get_mut(&output.id())
                    .ok_or_else(protocol_error)?
                    .dimensions = Some((width, height));
            }
            Incoming::OutputRemoved(proxy) => {
                let removed = self
                    .outputs
                    .remove(&proxy.id())
                    .ok_or_else(protocol_error)?;
                if self.selected_output.as_ref() == Some(&proxy.id()) {
                    self.selected_output = None;
                    self.default_output = None;
                    self.selected_output_lost = true;
                    for window in self.windows.values_mut() {
                        if window.fullscreen {
                            window.fullscreen = false;
                            window.proposed_size = None;
                            window.awaiting_dimensions = false;
                            window.position = None;
                        }
                    }
                }
                removed.layer.destroy();
                proxy.destroy();
                self.flush_pending = true;
            }
            Incoming::SeatCreated(proxy) => {
                if self.seats.len() == MAX_BACKEND_SEATS {
                    proxy.destroy();
                    return Err(capacity(
                        BackendCapacityResource::Seats,
                        MAX_BACKEND_SEATS as u64,
                    ));
                }
                self.next_seat_ordinal = self
                    .next_seat_ordinal
                    .checked_add(1)
                    .ok_or_else(|| capacity(BackendCapacityResource::ObjectOrdinals, u64::MAX))?;
                if self.selected_seat.is_none() {
                    self.selected_seat = Some(proxy.id());
                    let layer_seat = self
                        .layer_shell
                        .as_ref()
                        .ok_or_else(protocol_error)?
                        .send_constructor::<RiverLayerShellSeatV1>(
                            river_layer_shell_v1::Request::GetSeat {
                                seat: proxy.clone(),
                            },
                            DirectData::new(ObjectKind::LayerSeat, &self.incoming_tx),
                        )
                        .map_err(invalid_id)?;
                    let xkb_seat = self
                        .xkb_bindings
                        .as_ref()
                        .ok_or_else(protocol_error)?
                        .send_constructor::<RiverXkbBindingsSeatV1>(
                            river_xkb_bindings_v1::Request::GetSeat {
                                seat: proxy.clone(),
                            },
                            DirectData::new(ObjectKind::XkbSeat, &self.incoming_tx),
                        )
                        .map_err(invalid_id)?;
                    for (id, spec) in &self.bindings {
                        let binding = self
                            .xkb_bindings
                            .as_ref()
                            .ok_or_else(protocol_error)?
                            .send_constructor::<RiverXkbBindingV1>(
                                river_xkb_bindings_v1::Request::GetXkbBinding {
                                    seat: proxy.clone(),
                                    keysym: keysym(&spec.keysym).ok_or_else(protocol_error)?,
                                    modifiers: WEnum::Value(
                                        river_seat_v1::Modifiers::from_bits_truncate(
                                            modifiers_bits(&spec.modifiers),
                                        ),
                                    ),
                                },
                                DirectData::new(ObjectKind::Binding(*id), &self.incoming_tx),
                            )
                            .map_err(invalid_id)?;
                        self.binding_objects.insert(
                            *id,
                            Binding {
                                proxy: binding,
                                enabled: false,
                            },
                        );
                    }
                    self.layer_seat = Some(layer_seat);
                    self.xkb_seat = Some(xkb_seat);
                    self.flush_pending = true;
                }
                self.seats.insert(
                    proxy.id(),
                    Seat {
                        proxy,
                        ordinal: self.next_seat_ordinal,
                    },
                );
            }
            Incoming::SeatRemoved(proxy) => {
                self.seats.remove(&proxy.id()).ok_or_else(protocol_error)?;
                if self.selected_seat.as_ref() == Some(&proxy.id()) {
                    self.selected_seat = None;
                    if let Some(layer_seat) = self.layer_seat.take() {
                        layer_seat.destroy();
                    }
                    if let Some(xkb_seat) = self.xkb_seat.take() {
                        xkb_seat.destroy();
                    }
                    for (_, binding) in std::mem::take(&mut self.binding_objects) {
                        binding.proxy.destroy();
                    }
                    self.terminal_after_response = Some(BackendError::Unavailable {
                        message: "selected River seat was removed".to_owned(),
                    });
                }
                proxy.destroy();
                self.flush_pending = true;
            }
            Incoming::WindowInteracted(window) => {
                let backend_id = self
                    .windows
                    .get(&window.id())
                    .and_then(|window| window.backend_id.clone())
                    .ok_or_else(protocol_error)?;
                self.policy_events
                    .push(BackendPolicyEvent::FocusChanged(Some(backend_id)));
            }
            Incoming::Workarea {
                output,
                x,
                y,
                width,
                height,
            } => {
                let Some((id, record)) = self
                    .outputs
                    .iter_mut()
                    .find(|(_, record)| record.layer.id() == output.id())
                else {
                    return Err(protocol_error());
                };
                let tiles = realm_core::layout::Rect::new(x, y, width, height);
                record.workarea = Some(tiles);
                if self.selected_output.as_ref() == Some(id) {
                    let (ox, oy) = record.position.ok_or_else(protocol_error)?;
                    let (ow, oh) = record.dimensions.ok_or_else(protocol_error)?;
                    self.policy_events.push(BackendPolicyEvent::WorkareaChanged(
                        realm_core::layout::Workarea {
                            output: realm_core::layout::Rect::new(ox, oy, ow, oh),
                            tiles,
                        },
                    ));
                }
            }
            Incoming::ExclusiveFocus(exclusive) => {
                self.exclusive_focus = exclusive;
                if exclusive {
                    self.focused_window = None;
                }
                self.policy_events
                    .push(BackendPolicyEvent::ExclusiveFocusChanged(exclusive));
            }
            Incoming::BindingPressed(id) => {
                if !self.binding_objects.contains_key(&id) {
                    return Err(protocol_error());
                }
                self.policy_events
                    .push(BackendPolicyEvent::BindingPressed(id));
            }
            Incoming::BindingReleased(id) => {
                if !self.binding_objects.contains_key(&id) {
                    return Err(protocol_error());
                }
                self.policy_events
                    .push(BackendPolicyEvent::BindingReleased(id));
            }
            Incoming::BindingRepeatStopped(id) => {
                if !self.binding_objects.contains_key(&id) {
                    return Err(protocol_error());
                }
                self.policy_events
                    .push(BackendPolicyEvent::BindingRepeatStopped(id));
            }
            Incoming::UnboundKeyEaten => {
                self.policy_events.push(BackendPolicyEvent::UnboundKeyEaten)
            }
            Incoming::ModifiersChanged { old, new } => {
                self.policy_events
                    .push(BackendPolicyEvent::ModifiersChanged {
                        old: decode_modifiers(old)?,
                        new: decode_modifiers(new)?,
                    })
            }
            Incoming::ManageStart => {
                self.turn_requested = false;
                if self
                    .pending_response
                    .as_ref()
                    .is_some_and(|pending| pending.stage == ResponseStage::AwaitCorrectionManage)
                {
                    let pending = self.pending_response.take().ok_or_else(protocol_error)?;
                    if pending.terminal_error.is_some() {
                        return Err(protocol_error());
                    }
                    if let Some(projection) = pending.response.projection {
                        self.last_projection = Some(projection);
                    }
                    self.deferred_corrections = pending.corrections;
                    self.public_events
                        .push_back(BackendEvent::OperationCompleted {
                            ticket: pending.ticket,
                            result: Ok(()),
                        });
                    self.drain_ticket = Some(pending.ticket);
                    self.finish_policy_turn()?;
                } else {
                    self.finish_policy_turn()?;
                }
            }
            Incoming::RenderStart => {
                if let Some(pending) = self.pending_response.as_ref() {
                    let projection = pending
                        .terminal_error
                        .is_none()
                        .then(|| {
                            pending
                                .response
                                .projection
                                .clone()
                                .or_else(|| self.last_projection.clone())
                        })
                        .flatten();
                    if pending.stage == ResponseStage::AwaitCorrectionManage {
                        let _ = self.apply_render_response(projection.as_deref(), false)?;
                        self.window_manager
                            .as_ref()
                            .ok_or_else(protocol_error)?
                            .render_finish();
                        self.flush_pending = true;
                        return Ok(());
                    }
                    if pending.stage != ResponseStage::AwaitRender {
                        return Err(protocol_error());
                    }
                    let allow_correction = !pending.correction_round;
                    let corrections =
                        self.apply_render_response(projection.as_deref(), allow_correction)?;
                    self.window_manager
                        .as_ref()
                        .ok_or_else(protocol_error)?
                        .render_finish();
                    let pending = self.pending_response.as_mut().ok_or_else(protocol_error)?;
                    if corrections.is_empty() {
                        pending.stage = ResponseStage::FlushingRender;
                    } else {
                        pending.corrections = corrections;
                        pending.correction_round = true;
                        pending.stage = ResponseStage::AwaitCorrectionManage;
                        self.window_manager
                            .as_ref()
                            .ok_or_else(protocol_error)?
                            .manage_dirty();
                    }
                    self.flush_pending = true;
                } else {
                    let projection = self.last_projection.clone();
                    let _ = self.apply_render_response(projection.as_deref(), false)?;
                    self.window_manager
                        .as_ref()
                        .ok_or_else(protocol_error)?
                        .render_finish();
                    self.flush_pending = true;
                }
            }
            incoming @ (Incoming::InputDeviceCreated(_)
            | Incoming::InputDeviceRemoved(_)
            | Incoming::InputDeviceType { .. }
            | Incoming::InputDeviceDone(_)
            | Incoming::LibinputDeviceCreated(_)
            | Incoming::LibinputDeviceRemoved(_)
            | Incoming::LibinputTapSupport { .. }
            | Incoming::LibinputTapCurrent { .. }
            | Incoming::LibinputDeviceDone(_)
            | Incoming::LibinputResult { .. }) => self.apply_input_event(incoming)?,
            Incoming::Global { .. }
            | Incoming::GlobalRemoved(_)
            | Incoming::SyncDone
            | Incoming::Malformed => return Err(protocol_error()),
        }
        self.validate_policy_capacity()?;
        Ok(())
    }

    fn validate_policy_capacity(&self) -> BackendResult<()> {
        let limit = if self.replay {
            MAX_REPLAY_POLICY_EVENTS
        } else {
            MAX_POLICY_EVENTS
        };
        if self.policy_events.len() > limit {
            return Err(capacity(BackendCapacityResource::PolicyFacts, limit as u64));
        }
        if self
            .policy_events
            .iter()
            .map(event_text_bytes)
            .sum::<usize>()
            > MAX_POLICY_TEXT_BYTES
        {
            return Err(capacity(
                BackendCapacityResource::PolicyTextBytes,
                MAX_POLICY_TEXT_BYTES as u64,
            ));
        }
        Ok(())
    }

    fn finish_policy_turn(&mut self) -> BackendResult<()> {
        let mut opened: Vec<_> = self
            .windows
            .values_mut()
            .filter(|window| window.pending_open)
            .collect();
        opened.sort_by_key(|window| window.ordinal);
        for window in opened {
            let backend_id = window.backend_id.clone().ok_or_else(protocol_error)?;
            self.policy_events.push(BackendPolicyEvent::WindowOpened {
                backend_id,
                app_id: window.app_id.clone(),
                title: window.title.clone(),
            });
            window.pending_open = false;
        }
        if self.selected_output.is_none() {
            let require_workarea = self.selected_output_lost;
            self.selected_output = self
                .outputs
                .iter()
                .filter(|(_, output)| {
                    output.position.is_some()
                        && output.dimensions.is_some()
                        && (!require_workarea || output.workarea.is_some())
                })
                .min_by_key(|(_, output)| output.ordinal)
                .map(|(id, _)| id.clone());
            self.selected_output_lost = false;
            if require_workarea {
                if let Some(output) = self
                    .selected_output
                    .as_ref()
                    .and_then(|id| self.outputs.get(id))
                {
                    let (x, y) = output.position.ok_or_else(protocol_error)?;
                    let (w, h) = output.dimensions.ok_or_else(protocol_error)?;
                    let tiles = output.workarea.ok_or_else(protocol_error)?;
                    self.policy_events.push(BackendPolicyEvent::WorkareaChanged(
                        realm_core::layout::Workarea {
                            output: realm_core::layout::Rect::new(x, y, w, h),
                            tiles,
                        },
                    ));
                }
            }
        }
        if self.selected_output.is_none() || self.selected_seat.is_none() {
            self.terminal_after_response = Some(BackendError::Unavailable {
                message: "River has no complete selected output or seat".to_owned(),
            });
        } else if let Some(selected) = self
            .selected_seat
            .as_ref()
            .and_then(|id| self.seats.get(id))
        {
            if self
                .seats
                .values()
                .any(|seat| seat.ordinal < selected.ordinal)
            {
                return Err(protocol_error());
            }
        }
        if self.replay {
            self.policy_events
                .push(BackendPolicyEvent::InitialReplayComplete);
        }
        let limit = if self.replay {
            MAX_REPLAY_POLICY_EVENTS
        } else {
            MAX_POLICY_EVENTS
        };
        if self.policy_events.len() > limit {
            return Err(capacity(BackendCapacityResource::PolicyFacts, limit as u64));
        }
        let text_bytes = self
            .policy_events
            .iter()
            .map(event_text_bytes)
            .sum::<usize>();
        if text_bytes > MAX_POLICY_TEXT_BYTES {
            return Err(capacity(
                BackendCapacityResource::PolicyTextBytes,
                MAX_POLICY_TEXT_BYTES as u64,
            ));
        }
        self.next_turn = self
            .next_turn
            .checked_add(1)
            .ok_or_else(|| capacity(BackendCapacityResource::PolicyTurnIds, u64::MAX))?;
        let id = BackendPolicyTurnId::new(self.next_turn).ok_or_else(protocol_error)?;
        let events = std::mem::take(&mut self.policy_events);
        self.open_turn = Some(id);
        self.public_events
            .push_back(BackendEvent::PolicyTurn(BackendPolicyTurn {
                id,
                drains: self.drain_ticket.take(),
                events,
            }));
        self.replay = false;
        Ok(())
    }

    fn apply_render_response(
        &mut self,
        projection: Option<&[realm_core::layout::Placement]>,
        allow_correction: bool,
    ) -> BackendResult<HashMap<ObjectId, ((i32, i32), (i32, i32))>> {
        let Some(projection) = projection else {
            return Ok(HashMap::new());
        };
        let mut corrections = HashMap::new();
        let visible: HashSet<_> = projection.iter().map(|placement| placement.win).collect();
        for window in self.windows.values_mut() {
            let Some(win) = window.assigned else {
                continue;
            };
            if !visible.contains(&win) && window.hidden != Some(true) {
                window.proxy.hide();
                window.hidden = Some(true);
            }
        }

        let raise = projection.iter().any(|placement| placement.occluded);
        for placement in projection {
            let backend_id = self
                .realm_ids
                .get(&placement.win)
                .ok_or_else(protocol_error)?;
            let object_id = self.window_ids.get(backend_id).ok_or_else(protocol_error)?;
            let window = self.windows.get_mut(object_id).ok_or_else(protocol_error)?;
            if !window.fullscreen {
                let position = (placement.rect.x, placement.rect.y);
                if window.position != Some(position) {
                    window.node.set_position(position.0, position.1);
                    window.position = Some(position);
                }
                if window.focused != Some(placement.focused) {
                    let (r, g, b, a) = border_rgba(placement.focused);
                    window.proxy.set_borders(
                        river_window_v1::Edges::from_bits_truncate(15),
                        1,
                        r,
                        g,
                        b,
                        a,
                    );
                    window.focused = Some(placement.focused);
                }
                let target = (placement.rect.w, placement.rect.h);
                if let Some(actual) = window.dimensions {
                    if actual.0 >= target.0 && actual.1 >= target.1 && actual != target {
                        window.proxy.set_content_clip_box(0, 0, target.0, target.1);
                    } else if actual == target {
                        window.proxy.set_content_clip_box(0, 0, 0, 0);
                    } else if allow_correction {
                        corrections.insert(
                            object_id.clone(),
                            (
                                target,
                                (
                                    target.0 + (target.0 - actual.0).max(0),
                                    target.1 + (target.1 - actual.1).max(0),
                                ),
                            ),
                        );
                    }
                }
                if window
                    .dimensions
                    .is_some_and(|actual| actual.0 >= target.0 && actual.1 >= target.1)
                    || !allow_correction && window.dimensions.is_some()
                {
                    if window.hidden != Some(false) {
                        window.proxy.show();
                        window.hidden = Some(false);
                    }
                }
                if raise && placement.focused {
                    window.node.place_top();
                }
            } else if window.hidden != Some(false) {
                window.proxy.show();
                window.hidden = Some(false);
            }
        }
        Ok(corrections)
    }

    fn request_policy_turn_inner(&mut self) -> BackendResult<()> {
        if !self.connected || self.exit_started {
            return Err(protocol_error());
        }
        if self.turn_requested {
            return Ok(());
        }
        self.window_manager
            .as_ref()
            .ok_or_else(protocol_error)?
            .send_request(river_window_manager_v1::Request::ManageDirty {})
            .map_err(invalid_id)?;
        self.turn_requested = true;
        self.flush_pending = true;
        Ok(())
    }

    fn begin_exit_session_inner(&mut self, policy: BackendExitPolicy) -> BackendResult<()> {
        if !self.connected || self.exit_started {
            return Err(protocol_error());
        }
        self.read_guard.take();
        self.public_events.clear();
        if self.open_turn.take().is_some() {
            if self.selected_output.is_some() && self.xkb_seat.is_some() {
                self.apply_manage_response(&BackendPolicyResponse {
                    projection: None,
                    closes: Vec::new(),
                    bindings: super::BackendBindingState {
                        enabled: policy.enabled,
                        watched_modifiers: policy.watched_modifiers,
                        next_key_edge: BackendNextKeyEdge::Cancel,
                    },
                })?;
            }
            self.window_manager
                .as_ref()
                .ok_or_else(protocol_error)?
                .manage_finish();
        }
        self.pending_response = None;
        self.window_manager
            .as_ref()
            .ok_or_else(protocol_error)?
            .exit_session();
        self.flush_pending = true;
        self.flush_blocked = false;
        self.exit_started = true;
        Ok(())
    }
}

impl WmBackend for RiverBackend {
    fn name(&self) -> &str {
        RiverBackend::name(self)
    }

    fn connect(&mut self) -> BackendResult<Capabilities> {
        RiverBackend::connect(self)
    }

    fn assign_window(
        &mut self,
        backend_id: &BackendWindowId,
        win: WinId,
    ) -> Result<(), BackendContractError> {
        RiverBackend::assign_window(self, backend_id, win)
    }

    fn configure_bindings(&mut self, bindings: Vec<BackendBindingSpec>) -> BackendResult<()> {
        RiverBackend::configure_bindings(self, bindings)
    }

    fn request_policy_turn(&mut self) -> BackendResult<()> {
        self.request_policy_turn_inner()
    }

    fn respond_policy_turn(
        &mut self,
        turn: BackendPolicyTurnId,
        ticket: BackendTicket,
        response: BackendPolicyResponse,
    ) -> BackendResult<BackendSubmission> {
        RiverBackend::respond_policy_turn(self, turn, ticket, response)
    }

    fn begin_exit_session(&mut self, policy: BackendExitPolicy) -> BackendResult<()> {
        self.begin_exit_session_inner(policy)
    }

    fn event_fd(&self) -> BorrowedFd<'_> {
        RiverBackend::event_fd(self)
    }

    fn poll_interest(&self) -> BackendPollInterest {
        RiverBackend::poll_interest(self)
    }

    fn service(
        &mut self,
        ready: BackendReady,
        now: Instant,
    ) -> BackendResult<Option<BackendEvent>> {
        RiverBackend::service(self, ready, now)
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
    kind: ObjectKind,
) -> BackendResult<I> {
    registry
        .send_constructor::<I>(
            wl_registry::Request::Bind {
                name: global.name,
                id: (I::interface(), version),
            },
            DirectData::new(kind, incoming),
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

fn capacity(resource: BackendCapacityResource, limit: u64) -> BackendError {
    BackendError::Capacity { resource, limit }
}

fn event_text_bytes(event: &BackendPolicyEvent) -> usize {
    match event {
        BackendPolicyEvent::WindowOpened {
            backend_id,
            app_id,
            title,
        } => backend_id.as_str().len() + app_id.len() + title.len(),
        BackendPolicyEvent::WindowClosed(backend_id)
        | BackendPolicyEvent::GeometryDrifted { backend_id, .. } => backend_id.as_str().len(),
        BackendPolicyEvent::TitleChanged { backend_id, title } => {
            backend_id.as_str().len() + title.len()
        }
        BackendPolicyEvent::FocusChanged(Some(backend_id)) => backend_id.as_str().len(),
        _ => 0,
    }
}

fn keysym(name: &str) -> Option<u32> {
    match name {
        "Return" => Some(0xff0d),
        "Escape" => Some(0xff1b),
        "question" => Some(u32::from(b'?')),
        text if text.len() == 1 && text.as_bytes()[0].is_ascii_alphanumeric() => {
            Some(u32::from(text.as_bytes()[0]))
        }
        _ => None,
    }
}

fn modifiers_bits(modifiers: &[BackendModifier]) -> u32 {
    modifiers.iter().fold(0, |bits, modifier| {
        bits | match modifier {
            BackendModifier::Shift => 1,
            BackendModifier::Control => 4,
            BackendModifier::Alt => 8,
            BackendModifier::Super => 64,
        }
    })
}

fn decode_modifiers(bits: u32) -> BackendResult<Vec<BackendModifier>> {
    if bits & !(1 | 4 | 8 | 64) != 0 {
        return Err(protocol_error());
    }
    let mut modifiers = Vec::new();
    for (bit, modifier) in [
        (1, BackendModifier::Shift),
        (4, BackendModifier::Control),
        (8, BackendModifier::Alt),
        (64, BackendModifier::Super),
    ] {
        if bits & bit != 0 {
            modifiers.push(modifier);
        }
    }
    Ok(modifiers)
}

fn border_rgba(focused: bool) -> (u32, u32, u32, u32) {
    let (red, green, blue, alpha_percent) = if focused {
        (0xa6_u64, 0x92_u64, 0xec_u64, 60_u64)
    } else {
        (0x66_u64, 0x74_u64, 0x8e_u64, 30_u64)
    };
    let alpha = u64::from(u32::MAX) * alpha_percent / 100;
    let channel = |value: u64| (alpha * value / 255) as u32;
    (channel(red), channel(green), channel(blue), alpha as u32)
}
