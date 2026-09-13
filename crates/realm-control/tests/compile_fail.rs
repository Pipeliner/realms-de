use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

#[test]
fn bound_endpoint_cannot_activate_twice() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/bound_endpoint_cannot_activate_twice.rs");
    cases.compile_fail("tests/ui/active_listener_fd_cannot_escape_server_boundary.rs");
    cases.compile_fail("tests/ui/test_peer_credentials_are_not_public.rs");
    cases.pass("tests/ui/client_types_are_send.rs");
}

#[test]
fn client_test_seams_are_not_public() {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/ui/client_test_seams/Cargo.toml");
    let target = tempfile::tempdir().expect("create Cargo target directory");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .arg("check")
        .arg("--frozen")
        .arg("--offline")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(&manifest)
        .env("CARGO_TARGET_DIR", target.path())
        .output()
        .expect("run Cargo against realm-control public API");

    let stderr = String::from_utf8(output.stderr).expect("compiler diagnostics are UTF-8");
    assert!(!output.status.success(), "test seam unexpectedly public");
    assert!(
        stderr.contains("error[E0433]")
            && stderr.contains("TestClientOperations")
            && stderr.contains("client_test_seams_are_not_public.rs"),
        "expected E0433 for inaccessible TestClientOperations, got:\n{stderr}"
    );
}
