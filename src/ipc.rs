use crate::process::ProcessId;

const CHANNEL_CAPACITY: usize = 4;
const EVENT_CAPACITY: usize = 8;
const MAX_MESSAGE: usize = 64;
const RING_CAPACITY: usize = 4;
const WAITERS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Closed,
    Empty,
    Full,
    TooLarge,
    BufferTooSmall,
    PermissionDenied,
    Revoked,
    WaitQueueFull,
    AlreadyWaiting,
    InvalidWaiter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitResult {
    Completed,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: u16,
    pub data: u64,
}

#[derive(Clone, Copy)]
struct Message {
    length: usize,
    bytes: [u8; MAX_MESSAGE],
}

impl Message {
    const EMPTY: Self = Self {
        length: 0,
        bytes: [0; MAX_MESSAGE],
    };
}

pub struct WaitQueue {
    waiters: [Option<u32>; WAITERS],
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            waiters: [None; WAITERS],
        }
    }

    pub fn enqueue(&mut self, waiter: u32) -> Result<(), Error> {
        if waiter == 0 {
            return Err(Error::InvalidWaiter);
        }
        if self.waiters.contains(&Some(waiter)) {
            return Err(Error::AlreadyWaiting);
        }
        let slot = self
            .waiters
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::WaitQueueFull)?;
        *slot = Some(waiter);
        Ok(())
    }

    pub fn wake_one(&mut self) -> Option<u32> {
        let slot = self.waiters.iter_mut().find(|entry| entry.is_some())?;
        slot.take()
    }

    pub fn contains(&self, waiter: u32) -> bool {
        self.waiters.contains(&Some(waiter))
    }

    pub fn len(&self) -> usize {
        self.waiters.iter().filter(|entry| entry.is_some()).count()
    }
}

pub struct Channel {
    queue: [Option<Message>; CHANNEL_CAPACITY],
    head: usize,
    tail: usize,
    length: usize,
    senders: usize,
    receivers: usize,
    send_waiters: WaitQueue,
    receive_waiters: WaitQueue,
}

impl Channel {
    pub const fn new() -> Self {
        Self {
            queue: [None; CHANNEL_CAPACITY],
            head: 0,
            tail: 0,
            length: 0,
            senders: 1,
            receivers: 1,
            send_waiters: WaitQueue::new(),
            receive_waiters: WaitQueue::new(),
        }
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if self.receivers == 0 {
            return Err(Error::Closed);
        }
        if bytes.len() > MAX_MESSAGE {
            return Err(Error::TooLarge);
        }
        if self.length == CHANNEL_CAPACITY {
            return Err(Error::Full);
        }
        let mut message = Message::EMPTY;
        message.length = bytes.len();
        message.bytes[..bytes.len()].copy_from_slice(bytes);
        self.queue[self.tail] = Some(message);
        self.tail = (self.tail + 1) % CHANNEL_CAPACITY;
        self.length += 1;
        let _ = self.receive_waiters.wake_one();
        Ok(())
    }

    pub fn receive(&mut self, output: &mut [u8]) -> Result<usize, Error> {
        if self.length == 0 {
            return if self.senders == 0 {
                Err(Error::Closed)
            } else {
                Err(Error::Empty)
            };
        }
        let message = self.queue[self.head].ok_or(Error::Empty)?;
        if output.len() < message.length {
            return Err(Error::BufferTooSmall);
        }
        output[..message.length].copy_from_slice(&message.bytes[..message.length]);
        self.queue[self.head] = None;
        self.head = (self.head + 1) % CHANNEL_CAPACITY;
        self.length -= 1;
        let _ = self.send_waiters.wake_one();
        Ok(message.length)
    }

    pub fn send_blocking(&mut self, waiter: u32, bytes: &[u8]) -> Result<WaitResult, Error> {
        match self.send(bytes) {
            Ok(()) => Ok(WaitResult::Completed),
            Err(Error::Full) => {
                self.send_waiters.enqueue(waiter)?;
                Ok(WaitResult::Blocked)
            }
            Err(error) => Err(error),
        }
    }

    pub fn receive_blocking(
        &mut self,
        waiter: u32,
        output: &mut [u8],
    ) -> Result<Option<usize>, Error> {
        match self.receive(output) {
            Ok(length) => Ok(Some(length)),
            Err(Error::Empty) => {
                self.receive_waiters.enqueue(waiter)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn close_sender(&mut self) {
        self.senders = 0;
        let _ = self.receive_waiters.wake_one();
    }

    pub fn close_receiver(&mut self) {
        self.receivers = 0;
        let _ = self.send_waiters.wake_one();
    }

    pub fn pending(&self) -> usize {
        self.length
    }
}

pub struct SharedRing {
    producer: ProcessId,
    consumer: ProcessId,
    slots: [[u8; MAX_MESSAGE]; RING_CAPACITY],
    lengths: [usize; RING_CAPACITY],
    head: usize,
    tail: usize,
    length: usize,
    active: bool,
    producer_waiters: WaitQueue,
    consumer_waiters: WaitQueue,
}

impl SharedRing {
    pub const fn new(producer: ProcessId, consumer: ProcessId) -> Self {
        Self {
            producer,
            consumer,
            slots: [[0; MAX_MESSAGE]; RING_CAPACITY],
            lengths: [0; RING_CAPACITY],
            head: 0,
            tail: 0,
            length: 0,
            active: true,
            producer_waiters: WaitQueue::new(),
            consumer_waiters: WaitQueue::new(),
        }
    }

    pub fn publish(&mut self, caller: ProcessId, bytes: &[u8]) -> Result<(), Error> {
        self.check_active()?;
        if caller != self.producer {
            return Err(Error::PermissionDenied);
        }
        if bytes.len() > MAX_MESSAGE {
            return Err(Error::TooLarge);
        }
        if self.length == RING_CAPACITY {
            return Err(Error::Full);
        }
        self.slots[self.tail][..bytes.len()].copy_from_slice(bytes);
        self.lengths[self.tail] = bytes.len();
        self.tail = (self.tail + 1) % RING_CAPACITY;
        self.length += 1;
        let _ = self.consumer_waiters.wake_one();
        Ok(())
    }

    pub fn consume(&mut self, caller: ProcessId, output: &mut [u8]) -> Result<usize, Error> {
        self.check_active()?;
        if caller != self.consumer {
            return Err(Error::PermissionDenied);
        }
        if self.length == 0 {
            return Err(Error::Empty);
        }
        let length = self.lengths[self.head];
        if output.len() < length {
            return Err(Error::BufferTooSmall);
        }
        output[..length].copy_from_slice(&self.slots[self.head][..length]);
        self.lengths[self.head] = 0;
        self.head = (self.head + 1) % RING_CAPACITY;
        self.length -= 1;
        let _ = self.producer_waiters.wake_one();
        Ok(length)
    }

    pub fn publish_blocking(
        &mut self,
        caller: ProcessId,
        waiter: u32,
        bytes: &[u8],
    ) -> Result<WaitResult, Error> {
        match self.publish(caller, bytes) {
            Ok(()) => Ok(WaitResult::Completed),
            Err(Error::Full) => {
                self.producer_waiters.enqueue(waiter)?;
                Ok(WaitResult::Blocked)
            }
            Err(error) => Err(error),
        }
    }

    pub fn consume_blocking(
        &mut self,
        caller: ProcessId,
        waiter: u32,
        output: &mut [u8],
    ) -> Result<Option<usize>, Error> {
        match self.consume(caller, output) {
            Ok(length) => Ok(Some(length)),
            Err(Error::Empty) => {
                self.consumer_waiters.enqueue(waiter)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn revoke(&mut self, caller: ProcessId) -> Result<(), Error> {
        self.check_active()?;
        if caller != self.producer && caller != self.consumer {
            return Err(Error::PermissionDenied);
        }
        self.active = false;
        Ok(())
    }

    pub fn pending(&self) -> usize {
        self.length
    }

    fn check_active(&self) -> Result<(), Error> {
        self.active.then_some(()).ok_or(Error::Revoked)
    }
}

pub struct EventQueue {
    entries: [Option<Event>; EVENT_CAPACITY],
    head: usize,
    tail: usize,
    length: usize,
    closed: bool,
    waiters: WaitQueue,
}

impl EventQueue {
    pub const fn new() -> Self {
        Self {
            entries: [None; EVENT_CAPACITY],
            head: 0,
            tail: 0,
            length: 0,
            closed: false,
            waiters: WaitQueue::new(),
        }
    }

    pub fn push(&mut self, event: Event) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        if self.length == EVENT_CAPACITY {
            return Err(Error::Full);
        }
        self.entries[self.tail] = Some(event);
        self.tail = (self.tail + 1) % EVENT_CAPACITY;
        self.length += 1;
        let _ = self.waiters.wake_one();
        Ok(())
    }

    pub fn pop(&mut self) -> Result<Event, Error> {
        if self.length == 0 {
            return if self.closed {
                Err(Error::Closed)
            } else {
                Err(Error::Empty)
            };
        }
        let event = self.entries[self.head].ok_or(Error::Empty)?;
        self.entries[self.head] = None;
        self.head = (self.head + 1) % EVENT_CAPACITY;
        self.length -= 1;
        Ok(event)
    }

    pub fn pop_blocking(&mut self, waiter: u32) -> Result<Option<Event>, Error> {
        match self.pop() {
            Ok(event) => Ok(Some(event)),
            Err(Error::Empty) => {
                self.waiters.enqueue(waiter)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn close(&mut self) {
        self.closed = true;
        let _ = self.waiters.wake_one();
    }
}

pub fn contract_self_check() {
    let mut channel = Channel::new();
    assert_eq!(channel.send(b"hello"), Ok(()));
    let mut output = [0; MAX_MESSAGE];
    assert_eq!(channel.receive(&mut output), Ok(5));
    assert_eq!(&output[..5], b"hello");
    assert_eq!(channel.pending(), 0);
    assert_eq!(channel.receive_blocking(6, &mut output), Ok(None));
    channel.close_sender();
    assert_eq!(channel.receive(&mut output), Err(Error::Closed));
    channel.close_receiver();

    let mut blocked = Channel::new();
    for _ in 0..CHANNEL_CAPACITY {
        blocked.send(b"x").unwrap();
    }
    assert_eq!(blocked.send_blocking(7, b"x"), Ok(WaitResult::Blocked));
    assert_eq!(blocked.receive(&mut output), Ok(1));
    assert_eq!(blocked.send(b"x"), Ok(()));
    assert_eq!(blocked.send_blocking(7, b"x"), Ok(WaitResult::Blocked));

    let producer = ProcessId::INIT;
    let consumer = ProcessId::from_raw(2);
    let mut ring = SharedRing::new(producer, consumer);
    assert_eq!(ring.publish(consumer, b"bad"), Err(Error::PermissionDenied));
    assert_eq!(ring.publish(producer, b"ring"), Ok(()));
    assert_eq!(
        ring.consume(producer, &mut output),
        Err(Error::PermissionDenied)
    );
    assert_eq!(
        ring.consume(consumer, &mut output[..2]),
        Err(Error::BufferTooSmall)
    );
    assert_eq!(ring.consume(consumer, &mut output), Ok(4));
    assert_eq!(&output[..4], b"ring");
    assert_eq!(ring.pending(), 0);
    assert_eq!(ring.consume_blocking(consumer, 10, &mut output), Ok(None));
    for _ in 0..RING_CAPACITY {
        ring.publish(producer, b"x").unwrap();
    }
    assert_eq!(
        ring.publish_blocking(producer, 8, b"x"),
        Ok(WaitResult::Blocked)
    );
    ring.consume(consumer, &mut output).unwrap();
    ring.publish(producer, b"x").unwrap();
    assert_eq!(
        ring.revoke(ProcessId::from_raw(3)),
        Err(Error::PermissionDenied)
    );
    ring.revoke(producer).unwrap();
    assert_eq!(ring.publish(producer, b"x"), Err(Error::Revoked));

    let mut events = EventQueue::new();
    assert_eq!(events.pop_blocking(9), Ok(None));
    events.push(Event { kind: 1, data: 42 }).unwrap();
    assert_eq!(events.pop(), Ok(Event { kind: 1, data: 42 }));
    events.close();
    assert_eq!(events.pop(), Err(Error::Closed));

    let mut waiters = WaitQueue::new();
    assert_eq!(waiters.enqueue(0), Err(Error::InvalidWaiter));
    waiters.enqueue(11).unwrap();
    assert!(waiters.contains(11));
    assert_eq!(waiters.enqueue(11), Err(Error::AlreadyWaiting));
    assert_eq!(waiters.wake_one(), Some(11));
    assert_eq!(waiters.len(), 0);
}
