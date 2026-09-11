use std::os::fd::AsFd;
use std::path::Path;

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
    let _escaped = AsFd::as_fd(&listener);
}
