use std::path::Path;
use std::time::Instant;

use realm_control::test_runtime_dir;

fn main() {
    let listener = test_runtime_dir(Path::new("/tmp"))
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap()
        .activate()
        .unwrap();
    let _server = listener.into_server_with_test_credentials(Instant::now(), []);
}
