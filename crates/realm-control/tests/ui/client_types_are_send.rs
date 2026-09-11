use realm_control::{Client, Subscription};

fn assert_send<T: Send>() {}

fn main() {
    assert_send::<Client>();
    assert_send::<Subscription>();
}
