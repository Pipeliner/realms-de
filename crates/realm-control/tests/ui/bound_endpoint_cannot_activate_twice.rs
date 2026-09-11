use std::path::Path;

use realm_control::test_runtime_dir;

fn main() {
    let bound = test_runtime_dir(Path::new("/tmp"))
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let _active = bound.activate().unwrap();
    let _second_active = bound.activate().unwrap();
}
