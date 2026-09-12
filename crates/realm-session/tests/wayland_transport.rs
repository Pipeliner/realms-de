use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use wayland_backend::client::{
    bounded_syscall_attempts_for_test, Backend, DispatchOne, FlushOnce, ObjectData, ObjectId,
    ReadOnce,
};
use wayland_backend::protocol::{Argument, Message};
use wayland_backend::smallvec::smallvec;

#[derive(Debug, Default)]
struct Counter(AtomicUsize);

impl ObjectData for Counter {
    fn event(
        self: Arc<Self>,
        _backend: &Backend,
        _msg: Message<ObjectId, OwnedFd>,
    ) -> Option<Arc<dyn ObjectData>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        None
    }

    fn destroyed(&self, _object_id: ObjectId) {}
}

fn connection() -> (Backend, UnixStream) {
    let (client, server) = UnixStream::pair().unwrap();
    (Backend::connect(client).unwrap(), server)
}

fn new_callback(backend: &Backend, data: Arc<Counter>) -> ObjectId {
    backend
        .send_request(
            Message {
                sender_id: backend.display_id(),
                opcode: 0,
                args: smallvec![Argument::NewId(ObjectId::null())],
            },
            Some(data),
            None,
        )
        .unwrap()
}

fn callback_done(id: &ObjectId, value: u32) -> [u8; 12] {
    let mut frame = [0; 12];
    frame[0..4].copy_from_slice(&id.protocol_id().to_ne_bytes());
    frame[4..8].copy_from_slice(&((12_u32 << 16) | 0).to_ne_bytes());
    frame[8..12].copy_from_slice(&value.to_ne_bytes());
    frame
}

#[test]
fn one_ingress_and_one_protocol_callback_are_separate_bounded_steps() {
    let (backend, mut peer) = connection();
    let first = Arc::new(Counter::default());
    let second = Arc::new(Counter::default());
    let first_id = new_callback(&backend, first.clone());
    let second_id = new_callback(&backend, second.clone());
    let mut input = Vec::from(callback_done(&first_id, 1));
    input.extend_from_slice(&callback_done(&second_id, 2));
    peer.write_all(&input).unwrap();

    let read = backend.prepare_read_bounded().unwrap().read_once().unwrap();
    assert_eq!(read, ReadOnce::Read { bytes: 24, fds: 0 });
    assert_eq!(
        backend.dispatch_one_pending().unwrap(),
        DispatchOne::Dispatched
    );
    assert_eq!(first.0.load(Ordering::SeqCst), 1);
    assert_eq!(second.0.load(Ordering::SeqCst), 0);
    assert_eq!(
        backend.dispatch_one_pending().unwrap(),
        DispatchOne::Dispatched
    );
    assert_eq!(second.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        backend.dispatch_one_pending().unwrap(),
        DispatchOne::NeedRead
    );
}

#[test]
fn stock_read_api_retains_its_upstream_drain_behavior() {
    let (backend, mut peer) = connection();
    let first = Arc::new(Counter::default());
    let second = Arc::new(Counter::default());
    let first_id = new_callback(&backend, first.clone());
    let second_id = new_callback(&backend, second.clone());
    let mut input = Vec::from(callback_done(&first_id, 1));
    input.extend_from_slice(&callback_done(&second_id, 2));
    peer.write_all(&input).unwrap();

    assert_eq!(backend.prepare_read().unwrap().read().unwrap(), 2);
    assert_eq!(first.0.load(Ordering::SeqCst), 1);
    assert_eq!(second.0.load(Ordering::SeqCst), 1);
}

#[test]
fn ingress_is_capped_under_continuous_peer_output() {
    let (backend, mut peer) = connection();
    peer.write_all(&vec![0_u8; 32_768]).unwrap();

    let read = backend.prepare_read_bounded().unwrap().read_once().unwrap();
    assert_eq!(
        read,
        ReadOnce::Read {
            bytes: 16_384,
            fds: 0
        }
    );
}

#[test]
fn partial_frame_continues_across_ingress_quanta() {
    let (backend, mut peer) = connection();
    let counter = Arc::new(Counter::default());
    let callback = new_callback(&backend, counter.clone());
    let frame = callback_done(&callback, 7);

    peer.write_all(&frame[..6]).unwrap();
    assert_eq!(
        backend.prepare_read_bounded().unwrap().read_once().unwrap(),
        ReadOnce::Read { bytes: 6, fds: 0 }
    );
    assert_eq!(
        backend.dispatch_one_pending().unwrap(),
        DispatchOne::NeedRead
    );
    peer.write_all(&frame[6..]).unwrap();
    assert_eq!(
        backend.prepare_read_bounded().unwrap().read_once().unwrap(),
        ReadOnce::Read { bytes: 6, fds: 0 }
    );
    assert_eq!(
        backend.dispatch_one_pending().unwrap(),
        DispatchOne::Dispatched
    );
    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
}

#[test]
fn prepared_read_has_exclusive_cancel_or_consume_ownership() {
    let (backend, _peer) = connection();
    let guard = backend.prepare_read_bounded().unwrap();
    assert!(backend.prepare_read_bounded().is_none());
    assert_eq!(
        backend.dispatch_one_pending().unwrap(),
        DispatchOne::PreparedReadLive
    );
    drop(guard);
    let guard = backend.prepare_read_bounded().unwrap();
    assert_eq!(guard.read_once().unwrap(), ReadOnce::WouldBlock);
    assert!(backend.prepare_read_bounded().is_some());
}

#[test]
fn bounded_prepared_read_refuses_stock_preparation() {
    let (backend, _peer) = connection();
    let guard = backend.prepare_read_bounded().unwrap();

    assert!(backend.prepare_read().is_none());

    drop(guard);
    assert!(backend.prepare_read().is_some());
}

#[test]
fn complete_frame_too_short_for_signature_is_fatal() {
    let (backend, mut peer) = connection();
    let callback = new_callback(&backend, Arc::new(Counter::default()));
    let mut invalid_frame = [0_u8; 8];
    invalid_frame[..4].copy_from_slice(&callback.protocol_id().to_ne_bytes());
    invalid_frame[4..].copy_from_slice(&(8_u32 << 16).to_ne_bytes());
    peer.write_all(&invalid_frame).unwrap();

    assert_eq!(
        backend.prepare_read_bounded().unwrap().read_once().unwrap(),
        ReadOnce::Read { bytes: 8, fds: 0 }
    );
    assert!(matches!(
        backend.dispatch_one_pending(),
        Err(wayland_backend::client::WaylandError::Protocol(_))
    ));
}

#[test]
fn bounded_syscall_does_not_retry_interrupted_operation() {
    assert_eq!(bounded_syscall_attempts_for_test(), 1);
}

#[test]
fn flush_once_surfaces_would_block_without_retrying() {
    let (backend, peer) = connection();
    rustix::net::sockopt::set_socket_recv_buffer_size(&peer, 4_096).unwrap();
    for _ in 0..50_000 {
        new_callback(&backend, Arc::new(Counter::default()));
    }

    let mut blocked = false;
    for _ in 0..512 {
        match backend.flush_once().unwrap() {
            FlushOnce::WouldBlock => {
                blocked = true;
                break;
            }
            FlushOnce::Progress { bytes } => assert!((1..=4_096).contains(&bytes)),
            FlushOnce::Complete => break,
        }
    }
    assert!(
        blocked,
        "peer that never reads must eventually backpressure one sendmsg"
    );
}
