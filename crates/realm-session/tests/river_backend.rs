use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::thread;
use std::time::Instant;

use realm_core::WinId;
use realm_session::backend::{
    BackendBindingState, BackendEvent, BackendNextKeyEdge, BackendPolicyEvent,
    BackendPolicyResponse, BackendReady, BackendSubmission, BackendTicket, BackendWindowId,
    RiverBackend, WmBackend,
};

const REQUIRED_GLOBALS: [(&str, u32); 5] = [
    ("river_window_manager_v1", 5),
    ("river_xkb_bindings_v1", 3),
    ("river_layer_shell_v1", 1),
    ("river_input_manager_v1", 2),
    ("river_libinput_config_v1", 2),
];

#[test]
fn river_backend_implements_the_daemon_backend_seam() {
    fn assert_backend<T: WmBackend>() {}
    assert_backend::<RiverBackend>();
}

fn frame(sender: u32, opcode: u16, body: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + body.len()).unwrap();
    let mut frame = Vec::with_capacity(size as usize);
    frame.extend_from_slice(&sender.to_ne_bytes());
    frame.extend_from_slice(&((size << 16) | u32::from(opcode)).to_ne_bytes());
    frame.extend_from_slice(body);
    frame
}

fn wire_string(text: &str) -> Vec<u8> {
    let len = u32::try_from(text.len() + 1).unwrap();
    let mut encoded = Vec::from(len.to_ne_bytes());
    encoded.extend_from_slice(text.as_bytes());
    encoded.push(0);
    while encoded.len() % 4 != 0 {
        encoded.push(0);
    }
    encoded
}

fn registry_global(registry: u32, name: u32, interface: &str, version: u32) -> Vec<u8> {
    let mut body = Vec::from(name.to_ne_bytes());
    body.extend_from_slice(&wire_string(interface));
    body.extend_from_slice(&version.to_ne_bytes());
    frame(registry, 0, &body)
}

fn callback_done(callback: u32) -> Vec<u8> {
    frame(callback, 0, &0_u32.to_ne_bytes())
}

fn read_request(peer: &mut UnixStream) -> (u32, u16, Vec<u8>) {
    let mut header = [0_u8; 8];
    peer.read_exact(&mut header).unwrap();
    let sender = u32::from_ne_bytes(header[..4].try_into().unwrap());
    let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
    let opcode = word as u16;
    let mut body = vec![0; (word >> 16) as usize - 8];
    peer.read_exact(&mut body).unwrap();
    (sender, opcode, body)
}

fn parse_wire_string(body: &[u8], offset: usize) -> (String, usize) {
    let len = u32::from_ne_bytes(body[offset..offset + 4].try_into().unwrap()) as usize;
    let end = offset + 4 + len;
    let text = String::from_utf8(body[offset + 4..end - 1].to_vec()).unwrap();
    (text, end.next_multiple_of(4))
}

#[derive(Debug, Default)]
struct BootstrapObserved {
    bindings: Vec<String>,
    repeat: Option<(i32, i32)>,
    tap_enabled: bool,
}

fn serve_bootstrap(mut peer: UnixStream, with_devices: bool) -> BootstrapObserved {
    let mut registry = None;
    let mut observed = BootstrapObserved::default();
    let mut input_device = None;
    let mut libinput_device = None;
    loop {
        let (sender, opcode, body) = read_request(&mut peer);
        if sender == 1 && opcode == 1 {
            let id = u32::from_ne_bytes(body[..4].try_into().unwrap());
            registry = Some(id);
        } else if sender == 1 && opcode == 0 {
            let callback = u32::from_ne_bytes(body[..4].try_into().unwrap());
            if observed.bindings.is_empty() {
                let registry_id = registry.unwrap();
                for (index, (interface, version)) in REQUIRED_GLOBALS.iter().enumerate() {
                    peer.write_all(&registry_global(
                        registry_id,
                        u32::try_from(index + 1).unwrap(),
                        interface,
                        *version,
                    ))
                    .unwrap();
                }
            }
            peer.write_all(&callback_done(callback)).unwrap();
        } else if Some(sender) == registry && opcode == 0 {
            let (interface, offset) = parse_wire_string(&body, 4);
            let object = u32::from_ne_bytes(body[offset + 4..offset + 8].try_into().unwrap());
            observed.bindings.push(interface.clone());
            if with_devices && interface == "river_input_manager_v1" {
                let device = 0xff00_0000_u32;
                input_device = Some(device);
                peer.write_all(&frame(object, 1, &device.to_ne_bytes()))
                    .unwrap();
                peer.write_all(&frame(device, 1, &0_u32.to_ne_bytes()))
                    .unwrap();
                peer.write_all(&frame(device, 3, &[])).unwrap();
            }
            if with_devices && interface == "river_libinput_config_v1" {
                let device = 0xff00_0001_u32;
                libinput_device = Some(device);
                peer.write_all(&frame(object, 1, &device.to_ne_bytes()))
                    .unwrap();
                peer.write_all(&frame(device, 5, &2_i32.to_ne_bytes()))
                    .unwrap();
                peer.write_all(&frame(device, 7, &0_u32.to_ne_bytes()))
                    .unwrap();
                peer.write_all(&frame(device, 55, &[])).unwrap();
            }
            if interface == "river_window_manager_v1" {
                return observed;
            }
        } else if Some(sender) == input_device && opcode == 2 {
            observed.repeat = Some((
                i32::from_ne_bytes(body[..4].try_into().unwrap()),
                i32::from_ne_bytes(body[4..8].try_into().unwrap()),
            ));
        } else if Some(sender) == libinput_device && opcode == 2 {
            let result = u32::from_ne_bytes(body[..4].try_into().unwrap());
            observed.tap_enabled = u32::from_ne_bytes(body[4..8].try_into().unwrap()) == 1;
            peer.write_all(&frame(result, 0, &[])).unwrap();
        }
    }
}

#[test]
fn river_backend_owns_the_real_wayland_socket() {
    let (client, _server) = UnixStream::pair().unwrap();

    let backend = RiverBackend::from_socket(client).unwrap();

    assert_eq!(backend.name(), "river-v0.4.8");
    assert!(backend.event_fd().as_raw_fd() >= 0);
}

#[test]
fn connect_bootstraps_required_globals_in_accepted_order() {
    let (client, server) = UnixStream::pair().unwrap();
    let fixture = thread::spawn(|| serve_bootstrap(server, false));
    let mut backend = RiverBackend::from_socket(client).unwrap();

    let capabilities = backend.connect().unwrap();

    assert!(capabilities.exact_geometry);
    assert!(capabilities.server_side_borders);
    assert!(capabilities.hide_show);
    assert!(capabilities.explicit_ordering);
    assert!(capabilities.fullscreen);
    assert!(capabilities.unsupported.is_empty());
    assert_eq!(
        fixture.join().unwrap().bindings,
        [
            "river_layer_shell_v1",
            "river_xkb_bindings_v1",
            "river_input_manager_v1",
            "river_libinput_config_v1",
            "river_window_manager_v1",
        ]
    );
}

#[test]
fn connect_applies_done_gated_keyboard_and_tap_policy_before_window_management() {
    let (client, server) = UnixStream::pair().unwrap();
    let fixture = thread::spawn(|| serve_bootstrap(server, true));
    let mut backend = RiverBackend::from_socket(client).unwrap();

    backend.connect().unwrap();

    let observed = fixture.join().unwrap();
    assert_eq!(observed.repeat, Some((25, 600)));
    assert!(observed.tap_enabled);
    assert_eq!(
        observed.bindings.last().map(String::as_str),
        Some("river_window_manager_v1")
    );
}

fn serve_initial_replay(mut peer: UnixStream, await_response: bool) -> Vec<u16> {
    let mut registry = None;
    let window_manager = loop {
        let (sender, opcode, body) = read_request(&mut peer);
        if sender == 1 && opcode == 1 {
            registry = Some(u32::from_ne_bytes(body[..4].try_into().unwrap()));
        } else if sender == 1 && opcode == 0 {
            let callback = u32::from_ne_bytes(body[..4].try_into().unwrap());
            if registry.is_some() {
                let registry_id = registry.unwrap();
                for (index, (interface, version)) in REQUIRED_GLOBALS.iter().enumerate() {
                    peer.write_all(&registry_global(
                        registry_id,
                        u32::try_from(index + 1).unwrap(),
                        interface,
                        *version,
                    ))
                    .unwrap();
                }
                registry = None;
            }
            peer.write_all(&callback_done(callback)).unwrap();
        } else if sender == 2 && opcode == 0 {
            let (interface, offset) = parse_wire_string(&body, 4);
            if interface == "river_window_manager_v1" {
                break u32::from_ne_bytes(body[offset + 4..offset + 8].try_into().unwrap());
            }
        }
    };

    let window = 0xff00_0000_u32;
    let output = 0xff00_0001_u32;
    let seat = 0xff00_0002_u32;
    peer.write_all(&frame(window_manager, 6, &window.to_ne_bytes()))
        .unwrap();
    peer.write_all(&frame(window, 3, &wire_string("foot")))
        .unwrap();
    peer.write_all(&frame(window, 4, &wire_string("shell")))
        .unwrap();
    peer.write_all(&frame(window, 17, &wire_string("window-1")))
        .unwrap();
    peer.write_all(&frame(window_manager, 7, &output.to_ne_bytes()))
        .unwrap();
    let mut position = Vec::from(0_i32.to_ne_bytes());
    position.extend_from_slice(&0_i32.to_ne_bytes());
    peer.write_all(&frame(output, 2, &position)).unwrap();
    let mut dimensions = Vec::from(1920_i32.to_ne_bytes());
    dimensions.extend_from_slice(&1080_i32.to_ne_bytes());
    peer.write_all(&frame(output, 3, &dimensions)).unwrap();
    peer.write_all(&frame(window_manager, 8, &seat.to_ne_bytes()))
        .unwrap();
    peer.write_all(&frame(window_manager, 2, &[])).unwrap();
    if !await_response {
        // Observe enough dependent constructors to keep the peer alive while
        // the backend exposes the replay turn; their exact catalogue is
        // asserted by the response fixture below.
        for _ in 0..3 {
            let _ = read_request(&mut peer);
        }
        return Vec::new();
    }
    let manage_finish = loop {
        let (sender, opcode, _) = read_request(&mut peer);
        if sender == window_manager && opcode == 2 {
            break opcode;
        }
    };
    peer.write_all(&frame(window_manager, 3, &[])).unwrap();
    let (_, render_finish, _) = read_request(&mut peer);
    vec![manage_finish, render_finish]
}

#[test]
fn initial_replay_is_one_bounded_report_order_policy_turn() {
    let (client, server) = UnixStream::pair().unwrap();
    let fixture = thread::spawn(|| serve_initial_replay(server, false));
    let mut backend = RiverBackend::from_socket(client).unwrap();
    backend.connect().unwrap();
    backend.configure_bindings(Vec::new()).unwrap();

    let mut observed = None;
    for _ in 0..64 {
        if let Some(event) = backend
            .service(
                BackendReady {
                    readable: true,
                    terminal: false,
                    writable: true,
                },
                Instant::now(),
            )
            .unwrap()
        {
            observed = Some(event);
            break;
        }
    }

    fixture.join().unwrap();
    let BackendEvent::PolicyTurn(turn) = observed.expect("initial replay turn") else {
        panic!("expected policy turn");
    };
    assert_eq!(turn.drains, None);
    assert_eq!(
        turn.events,
        vec![
            BackendPolicyEvent::WindowOpened {
                backend_id: BackendWindowId::new("window-1").unwrap(),
                app_id: "foot".to_owned(),
                title: "shell".to_owned(),
            },
            BackendPolicyEvent::InitialReplayComplete,
        ]
    );
}

#[test]
fn replay_response_finishes_manage_then_render_before_completion_and_drain() {
    let (client, server) = UnixStream::pair().unwrap();
    let fixture = thread::spawn(|| serve_initial_replay(server, true));
    let mut backend = RiverBackend::from_socket(client).unwrap();
    backend.connect().unwrap();
    backend.configure_bindings(Vec::new()).unwrap();

    let turn = loop {
        if let Some(BackendEvent::PolicyTurn(turn)) = backend
            .service(
                BackendReady {
                    readable: true,
                    terminal: false,
                    writable: true,
                },
                Instant::now(),
            )
            .unwrap()
        {
            break turn;
        }
    };
    backend
        .assign_window(&BackendWindowId::new("window-1").unwrap(), WinId(1))
        .unwrap();
    let ticket = BackendTicket::new(9).unwrap();
    assert_eq!(
        backend
            .respond_policy_turn(
                turn.id,
                ticket,
                BackendPolicyResponse {
                    projection: None,
                    closes: Vec::new(),
                    bindings: BackendBindingState {
                        enabled: Vec::new(),
                        watched_modifiers: Vec::new(),
                        next_key_edge: BackendNextKeyEdge::Preserve,
                    },
                },
            )
            .unwrap(),
        BackendSubmission::Pending
    );

    let mut events = Vec::new();
    for _ in 0..128 {
        if let Some(event) = backend
            .service(
                BackendReady {
                    readable: true,
                    terminal: false,
                    writable: true,
                },
                Instant::now(),
            )
            .unwrap()
        {
            events.push(event);
            if events.len() == 2 {
                break;
            }
        }
    }
    assert!(matches!(
        events.first(),
        Some(BackendEvent::OperationCompleted {
            ticket: completed,
            result: Ok(()),
        }) if *completed == ticket
    ));
    assert_eq!(
        events.get(1),
        Some(&BackendEvent::RetainedObservationsDrained { ticket })
    );
    assert_eq!(fixture.join().unwrap(), vec![2, 4]);
}
