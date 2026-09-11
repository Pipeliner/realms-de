#[test]
fn bound_endpoint_cannot_activate_twice() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/bound_endpoint_cannot_activate_twice.rs");
}
