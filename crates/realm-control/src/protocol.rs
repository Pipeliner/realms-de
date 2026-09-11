use std::mem;
use std::time::{Duration, Instant};

use realm_core::ipc::{self, Event, Request, Response, MAX_FRAME_BYTES, PROTOCOL_VERSION};
use realm_core::state::RealmState;

const HELLO_TIMEOUT: Duration = Duration::from_secs(1);
const FRAME_TIMEOUT: Duration = Duration::from_secs(2);
const OUTPUT_TIMEOUT: Duration = Duration::from_secs(2);
const TERMINAL_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConnectionPhase {
    AwaitHello,
    SendingHello,
    Ready,
    Subscriber,
    CloseAfterReply,
    Closing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MachineClose {
    InvalidInput,
    FrameTooLarge,
    ExcessPipeline,
    PeerClosed,
    Deadline,
    OutputComplete,
    SubscriberInput,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MachineAction {
    None,
    Request(Request),
    Close(MachineClose),
}

#[derive(Debug, Default)]
pub(crate) struct InputBuffer {
    bytes: Vec<u8>,
}

enum InputStep {
    Partial,
    Frame(Vec<u8>),
    TooLarge,
}

impl InputBuffer {
    fn push(&mut self, byte: u8) -> InputStep {
        self.bytes.push(byte);
        if byte == b'\n' {
            return InputStep::Frame(mem::take(&mut self.bytes));
        }
        if self.bytes.len() == MAX_FRAME_BYTES {
            return InputStep::TooLarge;
        }
        InputStep::Partial
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn clear(&mut self) {
        self.bytes.clear();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputClass {
    Hello,
    Ordinary,
    InitialState,
    State,
    Shutdown,
    Terminal,
}

#[derive(Debug)]
pub(crate) struct OutputCursor {
    bytes: Vec<u8>,
    offset: usize,
    class: OutputClass,
}

impl OutputCursor {
    fn new(bytes: Vec<u8>, class: OutputClass) -> Self {
        Self {
            bytes,
            offset: 0,
            class,
        }
    }

    fn remaining(&self) -> &[u8] {
        &self.bytes[self.offset..]
    }

    fn is_unstarted(&self) -> bool {
        self.offset == 0
    }

    fn advance(&mut self, count: usize) -> bool {
        assert!(count <= self.remaining().len());
        self.offset += count;
        self.offset == self.bytes.len()
    }
}

enum DecodedFrame {
    Request(Request),
    ValidNonRequest,
    Invalid,
}

enum RetainedFrame {
    Request(Request),
    TerminalError(&'static str),
}

pub(crate) struct ConnectionMachine {
    phase: ConnectionPhase,
    input: InputBuffer,
    output: Option<OutputCursor>,
    latest: Option<OutputCursor>,
    retained: Option<RetainedFrame>,
    request_pending: bool,
    pending_subscribe: bool,
    read_half_closed: bool,
    shutting_down: bool,
    session: String,
    hello_deadline: Instant,
    partial_deadline: Option<Instant>,
    output_deadline: Option<Instant>,
    close_deadline: Option<Instant>,
    shutdown_deadline: Option<Instant>,
}

impl ConnectionMachine {
    pub(crate) fn new(now: Instant, session: &str) -> Self {
        Self {
            phase: ConnectionPhase::AwaitHello,
            input: InputBuffer::default(),
            output: None,
            latest: None,
            retained: None,
            request_pending: false,
            pending_subscribe: false,
            read_half_closed: false,
            shutting_down: false,
            session: session.to_owned(),
            hello_deadline: now + HELLO_TIMEOUT,
            partial_deadline: None,
            output_deadline: None,
            close_deadline: None,
            shutdown_deadline: None,
        }
    }

    pub(crate) fn phase(&self) -> ConnectionPhase {
        self.phase
    }

    pub(crate) fn input_enabled(&self) -> bool {
        if self.read_half_closed || self.shutting_down {
            return false;
        }
        match self.phase {
            ConnectionPhase::AwaitHello => true,
            ConnectionPhase::Ready => !self.request_pending && self.output.is_none(),
            ConnectionPhase::Subscriber => true,
            _ => false,
        }
    }

    pub(crate) fn output(&self) -> Option<&[u8]> {
        self.output.as_ref().map(OutputCursor::remaining)
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        if self.phase == ConnectionPhase::Closing {
            return None;
        }
        let mut deadline = self.shutdown_deadline;
        for candidate in [
            (self.phase == ConnectionPhase::AwaitHello).then_some(self.hello_deadline),
            self.partial_deadline,
            self.output_deadline,
            self.close_deadline,
        ]
        .into_iter()
        .flatten()
        {
            deadline = Some(deadline.map_or(candidate, |current| current.min(candidate)));
        }
        deadline
    }

    pub(crate) fn expire(&mut self, now: Instant) -> MachineAction {
        match self.next_deadline() {
            Some(deadline) if now >= deadline => self.close(MachineClose::Deadline),
            _ => MachineAction::None,
        }
    }

    pub(crate) fn ingest(&mut self, now: Instant, bytes: &[u8]) -> MachineAction {
        if bytes.is_empty() {
            return MachineAction::None;
        }
        if self.phase == ConnectionPhase::Subscriber {
            return self.close(MachineClose::SubscriberInput);
        }
        if !matches!(
            self.phase,
            ConnectionPhase::AwaitHello | ConnectionPhase::SendingHello | ConnectionPhase::Ready
        ) || self.request_pending
            || self.read_half_closed
            || self.shutting_down
        {
            return self.close(MachineClose::ExcessPipeline);
        }

        let mut action = MachineAction::None;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if self.phase == ConnectionPhase::SendingHello && self.retained.is_some() {
                return self.close(MachineClose::ExcessPipeline);
            }
            if self.input.is_empty()
                && matches!(
                    self.phase,
                    ConnectionPhase::Ready | ConnectionPhase::SendingHello
                )
            {
                self.partial_deadline.get_or_insert(now + FRAME_TIMEOUT);
            }

            match self.input.push(byte) {
                InputStep::Partial => {}
                InputStep::TooLarge => return self.close(MachineClose::FrameTooLarge),
                InputStep::Frame(frame) => {
                    self.partial_deadline = None;
                    match self.phase {
                        ConnectionPhase::AwaitHello => match decode_frame(&frame) {
                            DecodedFrame::Request(Request::Hello { version, .. }) => {
                                self.queue_hello(now, version == PROTOCOL_VERSION);
                                if version != PROTOCOL_VERSION {
                                    break;
                                }
                            }
                            DecodedFrame::Request(_) => {
                                self.queue_terminal_error(now, "expected Hello");
                                break;
                            }
                            DecodedFrame::ValidNonRequest => {
                                self.queue_terminal_error(now, "invalid request");
                                break;
                            }
                            DecodedFrame::Invalid => {
                                return self.close(MachineClose::InvalidInput);
                            }
                        },
                        ConnectionPhase::SendingHello => {
                            self.retained = Some(match decode_frame(&frame) {
                                DecodedFrame::Request(Request::Hello { .. }) => {
                                    RetainedFrame::TerminalError("duplicate Hello")
                                }
                                DecodedFrame::Request(request) => RetainedFrame::Request(request),
                                DecodedFrame::ValidNonRequest => {
                                    RetainedFrame::TerminalError("invalid request")
                                }
                                DecodedFrame::Invalid => {
                                    return self.close(MachineClose::InvalidInput);
                                }
                            });
                            if index + 1 != bytes.len() {
                                return self.close(MachineClose::ExcessPipeline);
                            }
                        }
                        ConnectionPhase::Ready => match decode_frame(&frame) {
                            DecodedFrame::Request(Request::Hello { .. }) => {
                                self.queue_terminal_error(now, "duplicate Hello");
                                break;
                            }
                            DecodedFrame::Request(request) => {
                                self.request_pending = true;
                                self.pending_subscribe = request == Request::Subscribe;
                                action = MachineAction::Request(request);
                                if index + 1 != bytes.len() {
                                    return self.close(MachineClose::ExcessPipeline);
                                }
                            }
                            DecodedFrame::ValidNonRequest => {
                                self.queue_terminal_error(now, "invalid request");
                                break;
                            }
                            DecodedFrame::Invalid => {
                                return self.close(MachineClose::InvalidInput);
                            }
                        },
                        _ => return self.close(MachineClose::ExcessPipeline),
                    }
                }
            }
        }
        action
    }

    pub(crate) fn read_eof(&mut self, _now: Instant) -> MachineAction {
        if self.read_half_closed {
            return MachineAction::None;
        }
        if !self.input.is_empty() {
            return self.close(MachineClose::PeerClosed);
        }
        self.read_half_closed = true;
        match self.phase {
            ConnectionPhase::AwaitHello => self.close(MachineClose::PeerClosed),
            ConnectionPhase::Ready if !self.request_pending && self.output.is_none() => {
                self.close(MachineClose::PeerClosed)
            }
            ConnectionPhase::Closing => MachineAction::None,
            _ => MachineAction::None,
        }
    }

    pub(crate) fn advance_output(&mut self, now: Instant, count: usize) -> MachineAction {
        let Some(output) = self.output.as_mut() else {
            return MachineAction::None;
        };
        let class = output.class;
        if count > 0 && self.phase != ConnectionPhase::CloseAfterReply && !self.shutting_down {
            self.output_deadline = Some(now + OUTPUT_TIMEOUT);
        }
        if !output.advance(count) {
            return MachineAction::None;
        }
        self.output = None;
        self.output_deadline = None;

        match class {
            OutputClass::Hello if self.phase == ConnectionPhase::CloseAfterReply => {
                self.close(MachineClose::OutputComplete)
            }
            OutputClass::Hello => {
                self.phase = ConnectionPhase::Ready;
                match self.retained.take() {
                    Some(RetainedFrame::Request(request)) => {
                        self.request_pending = true;
                        self.pending_subscribe = request == Request::Subscribe;
                        MachineAction::Request(request)
                    }
                    Some(RetainedFrame::TerminalError(message)) => {
                        self.queue_terminal_error(now, message);
                        MachineAction::None
                    }
                    None if self.read_half_closed => self.close(MachineClose::PeerClosed),
                    None => MachineAction::None,
                }
            }
            OutputClass::Ordinary => {
                if self.read_half_closed {
                    self.close(MachineClose::PeerClosed)
                } else {
                    MachineAction::None
                }
            }
            OutputClass::InitialState | OutputClass::State => self.finish_subscriber_output(now),
            OutputClass::Shutdown => self.close(MachineClose::Shutdown),
            OutputClass::Terminal => self.close(MachineClose::OutputComplete),
        }
    }

    pub(crate) fn complete_request(&mut self, now: Instant, response: &Response) -> MachineAction {
        if self.shutting_down || !self.request_pending || self.pending_subscribe {
            return MachineAction::None;
        }
        self.request_pending = false;
        match encode_response(response) {
            Some(bytes) => {
                self.set_current(bytes, OutputClass::Ordinary, now);
                MachineAction::None
            }
            None => self.close(MachineClose::FrameTooLarge),
        }
    }

    pub(crate) fn complete_subscribe(&mut self, now: Instant, state: &RealmState) -> MachineAction {
        if self.shutting_down || !self.request_pending || !self.pending_subscribe {
            return MachineAction::None;
        }
        self.request_pending = false;
        self.pending_subscribe = false;
        match encode_event(&Event::State(Box::new(state.clone()))) {
            Some(bytes) => {
                self.phase = ConnectionPhase::Subscriber;
                self.set_current(bytes, OutputClass::InitialState, now);
                MachineAction::None
            }
            None => self.close(MachineClose::FrameTooLarge),
        }
    }

    pub(crate) fn publish_state(&mut self, now: Instant, frame: &[u8]) -> MachineAction {
        if self.phase != ConnectionPhase::Subscriber || self.shutting_down {
            return MachineAction::None;
        }
        if frame.len() > MAX_FRAME_BYTES || frame.last() != Some(&b'\n') {
            return self.close(MachineClose::FrameTooLarge);
        }
        let queued = OutputCursor::new(frame.to_vec(), OutputClass::State);
        if self.output.is_some() {
            self.latest = Some(queued);
        } else {
            self.output = Some(queued);
            self.output_deadline = Some(now + OUTPUT_TIMEOUT);
        }
        MachineAction::None
    }

    pub(crate) fn begin_shutdown(&mut self, now: Instant) -> MachineAction {
        if self.shutdown_deadline.is_some() || self.phase == ConnectionPhase::Closing {
            return MachineAction::None;
        }
        if self.phase != ConnectionPhase::Subscriber {
            return self.close(MachineClose::Shutdown);
        }

        self.shutting_down = true;
        self.shutdown_deadline = Some(now + TERMINAL_TIMEOUT);
        self.latest = None;
        let Some(shutdown) = encode_event(&Event::Shutdown) else {
            return self.close(MachineClose::FrameTooLarge);
        };
        let shutdown = OutputCursor::new(shutdown, OutputClass::Shutdown);

        match self
            .output
            .as_ref()
            .map(|output| (output.class, output.is_unstarted()))
        {
            Some((OutputClass::InitialState, true)) => self.latest = Some(shutdown),
            Some((OutputClass::InitialState | OutputClass::State, false)) => {}
            Some((OutputClass::State, true)) => self.output = Some(shutdown),
            Some((OutputClass::Shutdown, _)) => {}
            Some(_) => self.output = Some(shutdown),
            None => self.output = Some(shutdown),
        }
        MachineAction::None
    }

    fn finish_subscriber_output(&mut self, now: Instant) -> MachineAction {
        if let Some(next) = self.latest.take() {
            self.output = Some(next);
            self.output_deadline = Some(now + OUTPUT_TIMEOUT);
            return MachineAction::None;
        }
        if self.shutting_down {
            return self.close(MachineClose::Shutdown);
        }
        MachineAction::None
    }

    fn queue_hello(&mut self, now: Instant, version_matches: bool) {
        let reply = Response::Hello {
            version: PROTOCOL_VERSION,
            session: self.session.clone(),
        };
        let Some(bytes) = encode_response(&reply) else {
            self.close(MachineClose::FrameTooLarge);
            return;
        };
        self.output = Some(OutputCursor::new(bytes, OutputClass::Hello));
        if version_matches {
            self.phase = ConnectionPhase::SendingHello;
            self.output_deadline = Some(now + OUTPUT_TIMEOUT);
        } else {
            self.phase = ConnectionPhase::CloseAfterReply;
            self.close_deadline = Some(now + TERMINAL_TIMEOUT);
        }
    }

    fn queue_terminal_error(&mut self, now: Instant, message: &str) {
        let reply = Response::Error {
            message: message.to_owned(),
        };
        let Some(bytes) = encode_response(&reply) else {
            self.close(MachineClose::FrameTooLarge);
            return;
        };
        self.input.clear();
        self.output = Some(OutputCursor::new(bytes, OutputClass::Terminal));
        self.phase = ConnectionPhase::CloseAfterReply;
        self.partial_deadline = None;
        self.output_deadline = None;
        self.close_deadline = Some(now + TERMINAL_TIMEOUT);
    }

    fn set_current(&mut self, bytes: Vec<u8>, class: OutputClass, now: Instant) {
        self.output = Some(OutputCursor::new(bytes, class));
        self.output_deadline = Some(now + OUTPUT_TIMEOUT);
    }

    fn close(&mut self, reason: MachineClose) -> MachineAction {
        self.phase = ConnectionPhase::Closing;
        self.input.clear();
        self.output = None;
        self.latest = None;
        self.retained = None;
        self.request_pending = false;
        self.pending_subscribe = false;
        self.partial_deadline = None;
        self.output_deadline = None;
        self.close_deadline = None;
        self.shutdown_deadline = None;
        MachineAction::Close(reason)
    }
}

fn encode_response(response: &Response) -> Option<Vec<u8>> {
    ipc::encode(response).ok().map(String::into_bytes)
}

fn encode_event(event: &Event) -> Option<Vec<u8>> {
    ipc::encode(event).ok().map(String::into_bytes)
}

fn decode_frame(frame: &[u8]) -> DecodedFrame {
    let Ok(text) = std::str::from_utf8(frame) else {
        return DecodedFrame::Invalid;
    };
    match ipc::decode::<Request>(text) {
        Ok(request) => DecodedFrame::Request(request),
        Err(realm_core::Error::Ipc(error)) if error.is_data() => DecodedFrame::ValidNonRequest,
        Err(_) => DecodedFrame::Invalid,
    }
}
