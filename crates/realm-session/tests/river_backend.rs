use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::thread;

use realm_session::backend::RiverBackend;

const REQUIRED_GLOBALS: [(&str, u32); 5] = [
    ("river_window_manager_v1", 5),
    ("river_xkb_bindings_v1", 3),
    ("river_layer_shell_v1", 1),
    ("river_input_manager_v1", 2),
    ("river_libinput_config_v1", 2),
];

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

fn serve_bootstrap(mut peer: UnixStream) -> Vec<String> {
    let mut registry = None;
    let mut bindings = Vec::new();
    loop {
        let (sender, opcode, body) = read_request(&mut peer);
        if sender == 1 && opcode == 1 {
            let id = u32::from_ne_bytes(body[..4].try_into().unwrap());
            registry = Some(id);
        } else if sender == 1 && opcode == 0 {
            let callback = u32::from_ne_bytes(body[..4].try_into().unwrap());
            if bindings.is_empty() {
                let registry = registry.unwrap();
                for (index, (interface, version)) in REQUIRED_GLOBALS.iter().enumerate() {
                    peer.write_all(&registry_global(
                        registry,
                        u32::try_from(index + 1).unwrap(),
                        interface,
                        *version,
                    ))
                    .unwrap();
                }
            }
            peer.write_all(&callback_done(callback)).unwrap();
        } else if Some(sender) == registry && opcode == 0 {
            let (interface, _) = parse_wire_string(&body, 4);
            bindings.push(interface);
            if bindings.last().unwrap() == "river_window_manager_v1" {
                return bindings;
            }
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
    let fixture = thread::spawn(|| serve_bootstrap(server));
    let mut backend = RiverBackend::from_socket(client).unwrap();

    let capabilities = backend.connect().unwrap();

    assert!(capabilities.exact_geometry);
    assert!(capabilities.server_side_borders);
    assert!(capabilities.hide_show);
    assert!(capabilities.explicit_ordering);
    assert!(capabilities.fullscreen);
    assert!(capabilities.unsupported.is_empty());
    assert_eq!(
        fixture.join().unwrap(),
        [
            "river_layer_shell_v1",
            "river_xkb_bindings_v1",
            "river_input_manager_v1",
            "river_libinput_config_v1",
            "river_window_manager_v1",
        ]
    );
}
