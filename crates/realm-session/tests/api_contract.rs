#[test]
fn session_update_api_has_only_the_accepted_closed_shape() {
    trybuild::TestCases::new().pass("tests/ui/session_update_closed.rs");
}

#[test]
fn legacy_backend_methods_are_absent_after_migration() {
    trybuild::TestCases::new().pass("tests/ui/wm_backend_accepted.rs");
}

#[test]
fn session_exposes_read_only_backend_poll_registration_seam() {
    trybuild::TestCases::new().pass("tests/ui/session_backend_poll.rs");
}
