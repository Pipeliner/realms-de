#[test]
fn bound_endpoint_cannot_activate_twice() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/bound_endpoint_cannot_activate_twice.rs");
    cases.compile_fail("tests/ui/active_listener_fd_cannot_escape_server_boundary.rs");
    cases.compile_fail("tests/ui/test_peer_credentials_are_not_public.rs");
    cases.compile_fail("tests/ui/client_test_seams_are_not_public.rs");
    cases.pass("tests/ui/client_types_are_send.rs");
}
