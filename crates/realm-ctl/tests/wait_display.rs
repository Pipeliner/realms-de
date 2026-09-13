use std::{
    fs,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

fn session_functions(runtime: &tempfile::TempDir) -> std::path::PathBuf {
    let source = fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/session/realm-session"),
    )
    .unwrap();
    let functions = source
        .split_once("\nmain() {\n")
        .expect("realm-session main boundary")
        .0;
    let path = runtime.path().join("session-functions.sh");
    fs::write(&path, functions).unwrap();
    path
}

fn wait_child_until(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            return None;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn read_request(peer: &mut UnixStream) -> (u32, u16, Vec<u8>) {
    let mut header = [0_u8; 8];
    peer.read_exact(&mut header)
        .expect("Wayland request header");
    let sender = u32::from_ne_bytes(header[..4].try_into().unwrap());
    let word = u32::from_ne_bytes(header[4..].try_into().unwrap());
    let mut body = vec![0; (word >> 16) as usize - 8];
    peer.read_exact(&mut body).expect("Wayland request body");
    (sender, word as u16, body)
}

fn callback_done(callback: u32) -> [u8; 12] {
    let mut frame = [0_u8; 12];
    frame[..4].copy_from_slice(&callback.to_ne_bytes());
    frame[4..8].copy_from_slice(&(12_u32 << 16).to_ne_bytes());
    frame[8..].copy_from_slice(&0_u32.to_ne_bytes());
    frame
}

fn accept_before(listener: &UnixListener, deadline: Instant) -> UnixStream {
    listener.set_nonblocking(true).unwrap();
    loop {
        match listener.accept() {
            Ok((peer, _)) => return peer,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "realmctl never connected");
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept Wayland probe: {error}"),
        }
    }
}

#[test]
fn wait_display_binds_registry_and_completes_real_roundtrip() {
    let runtime = tempfile::tempdir().unwrap();
    let display = "wayland-9";
    let listener = UnixListener::bind(runtime.path().join(display)).unwrap();
    let server = thread::spawn(move || {
        let mut peer = accept_before(&listener, Instant::now() + Duration::from_secs(1));
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let registry = read_request(&mut peer);
        let sync = read_request(&mut peer);
        assert_eq!((registry.0, registry.1), (1, 1));
        assert_eq!((sync.0, sync.1), (1, 0));
        assert_eq!(registry.2.len(), 4);
        assert_eq!(sync.2.len(), 4);
        let callback = u32::from_ne_bytes(sync.2[..4].try_into().unwrap());
        peer.write_all(&callback_done(callback)).unwrap();
        let mut eof = [0_u8; 1];
        assert_eq!(peer.read(&mut eof).unwrap(), 0);
    });

    let output = Command::new(env!("CARGO_BIN_EXE_realmctl"))
        .args(["wait-display", "--timeout", "1"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", display)
        .env_remove("WAYLAND_SOCKET")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "realmctl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    server.join().unwrap();
}

#[test]
fn wait_display_rejects_non_positive_or_non_finite_deadlines_without_connecting() {
    let runtime = tempfile::tempdir().unwrap();
    let display = "wayland-invalid-timeout";
    let listener = UnixListener::bind(runtime.path().join(display)).unwrap();
    listener.set_nonblocking(true).unwrap();

    for timeout in ["0", "-1", "NaN", "inf"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_realmctl"))
            .args(["wait-display", "--timeout", timeout])
            .env("XDG_RUNTIME_DIR", runtime.path())
            .env("WAYLAND_DISPLAY", display)
            .env_remove("WAYLAND_SOCKET")
            .spawn()
            .unwrap();
        let status = wait_child_until(&mut child, Instant::now() + Duration::from_millis(200));
        assert!(status.is_some(), "invalid timeout {timeout} blocked");
        assert!(!status.unwrap().success());
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "invalid timeout {timeout} connected to Wayland"
        );
    }
}

#[test]
fn wait_display_silent_server_exits_at_one_process_deadline() {
    let runtime = tempfile::tempdir().unwrap();
    let display = "wayland-silent";
    let listener = UnixListener::bind(runtime.path().join(display)).unwrap();
    let server = thread::spawn(move || {
        let mut peer = accept_before(&listener, Instant::now() + Duration::from_secs(1));
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        assert_eq!(
            (read_request(&mut peer).0, read_request(&mut peer).0),
            (1, 1)
        );
        let mut eof = [0_u8; 1];
        assert_eq!(peer.read(&mut eof).unwrap(), 0);
    });

    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_realmctl"))
        .args(["wait-display", "--timeout", "0.05"])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", display)
        .env_remove("WAYLAND_SOCKET")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let status = wait_child_until(&mut child, Instant::now() + Duration::from_millis(400));
    let elapsed = started.elapsed();

    assert!(status.is_some(), "realmctl outlived its process deadline");
    assert!(!status.unwrap().success());
    assert!(elapsed < Duration::from_millis(300), "elapsed {elapsed:?}");
    server.join().unwrap();
}

#[test]
fn session_wrapper_passes_only_discovery_deadline_remainder_to_probe() {
    let runtime = tempfile::tempdir().unwrap();
    let functions = session_functions(&runtime);
    let capture = runtime.path().join("timeout");
    let status = Command::new("/bin/bash")
        .arg("-c")
        .arg(
            r#"
                . "$1"
                log() { :; }
                wayland_sockets() { sleep 0.20; printf '%s\n' wayland-9; }
                realmctl() {
                    test "$1" = wait-display
                    test "$2" = --timeout
                    printf '%s\n' "$3" >"$CAPTURE"
                }
                REALM_CTL=realmctl
                pre_wayland=()
                sleep 5 &
                compositor_pid=$!
                trap 'kill "$compositor_pid" 2>/dev/null || true' EXIT
                wait_for_display
            "#,
        )
        .arg("bash")
        .arg(functions)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("REALM_WAIT_SECONDS", "1")
        .env("CAPTURE", &capture)
        .status()
        .unwrap();

    assert!(status.success());
    let remaining: f64 = fs::read_to_string(capture).unwrap().trim().parse().unwrap();
    assert!(remaining > 0.0, "remaining deadline was {remaining}");
    assert!(
        remaining < 0.9,
        "probe received a fresh deadline: {remaining}"
    );
}

#[test]
fn session_wrapper_aborts_probe_when_compositor_dies() {
    let runtime = tempfile::tempdir().unwrap();
    let functions = session_functions(&runtime);
    let mut child = Command::new("/bin/bash")
        .arg("-c")
        .arg(
            r#"
                . "$1"
                log() { :; }
                wayland_sockets() { printf '%s\n' wayland-9; }
                realmctl() { sleep 5; }
                REALM_CTL=realmctl
                pre_wayland=()
                sleep 0.05 &
                compositor_pid=$!
                wait_for_display
            "#,
        )
        .arg("bash")
        .arg(functions)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("REALM_WAIT_SECONDS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let started = Instant::now();
    let status = wait_child_until(&mut child, Instant::now() + Duration::from_millis(400));
    let elapsed = started.elapsed();
    assert!(status.is_some(), "wrapper ignored early compositor death");
    assert!(!status.unwrap().success());
    assert!(elapsed < Duration::from_millis(300), "elapsed {elapsed:?}");
}
