Task-1 evidence report — portable bounded control-frame encoder.

## RED

Command:
`cargo test -p realm-core ipc::tests::frame_encoder_accepts_exact_limit_and_rejects_next_byte -- --exact`

Observed output (first run before final boundary correction):

```
thread 'ipc::tests::frame_encoder_accepts_exact_limit_and_rejects_next_byte' (222) panicked at crates/realm-core/src/ipc.rs:254:44:
called `Result::unwrap()` on an `Err` value: IpcFrameTooLarge { limit: 65536 }
```

Command:
`cargo test -p realm-core ipc::tests::frame_encoder_stops_serialization_at_the_bound -- --exact`

Observed output (before fixture byte-count correction):

```
thread 'ipc::tests::frame_encoder_stops_serialization_at_the_bound' (13) panicked at crates/realm-core/src/ipc.rs:270:9:
assertion `left == right` failed
left: 65411
right: 65410
```

## GREEN

Command:
`cargo test -p realm-core ipc::tests::frame_encoder_accepts_exact_limit_and_rejects_next_byte -- --exact`

Observed output:

```
running 1 test
test ipc::tests::frame_encoder_accepts_exact_limit_and_rejects_next_byte ... ok
```

Command:
`cargo test -p realm-core ipc::tests::frame_encoder_stops_serialization_at_the_bound -- --exact`

Observed output:

```
running 1 test
test ipc::tests::frame_encoder_stops_serialization_at_the_bound ... ok
```

Command:
`cargo test -p realm-core`

Observed output:

```
running 58 tests
...
test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Command:
`cargo fmt --all -- --check`

Observed output:

```
```

No formatting differences after final implementation.
