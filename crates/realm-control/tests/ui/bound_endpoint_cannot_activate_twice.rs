use std::path::Path;

use realm_control::test_runtime_dir;

// Keep these source lines two digits wide: rustc 1.85 and current stable
// otherwise render their one-digit diagnostic gutters differently.
//

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
