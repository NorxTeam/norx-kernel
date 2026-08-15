use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};

const DATA_PORT: u16 = 0x60;
const STATUS_COMMAND_PORT: u16 = 0x64;

const OUTPUT_FULL: u8 = 1 << 0;
const INPUT_FULL: u8 = 1 << 1;
const AUX_OUTPUT: u8 = 1 << 5;

const FIRST_IRQ: u8 = 1 << 0;
const SECOND_IRQ: u8 = 1 << 1;
const TRANSLATION: u8 = 1 << 6;

const READ_CONFIG: u8 = 0x20;
const WRITE_CONFIG: u8 = 0x60;
const TEST_CONTROLLER: u8 = 0xaa;
const TEST_FIRST_PORT: u8 = 0xab;
const TEST_SECOND_PORT: u8 = 0xa9;
const DISABLE_FIRST_PORT: u8 = 0xad;
const DISABLE_SECOND_PORT: u8 = 0xa7;
const ENABLE_FIRST_PORT: u8 = 0xae;
const ENABLE_SECOND_PORT: u8 = 0xa8;
const WRITE_SECOND_PORT: u8 = 0xd4;

const CONTROLLER_OK: u8 = 0x55;
const PORT_OK: u8 = 0x00;
const ACK: u8 = 0xfa;
const RESEND: u8 = 0xfe;

const POLL_LIMIT: usize = 100_000;
const COMMAND_RETRIES: usize = 3;
const IRQ_DRAIN_LIMIT: usize = 8;
const QUEUE_CAPACITY: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Port {
    Keyboard,
    Mouse,
}

impl Port {
    fn is_mouse(self) -> bool {
        matches!(self, Self::Mouse)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InitResult {
    Ready,
    Unsupported,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Error {
    Timeout,
    ControllerSelfTest,
    PortSelfTest,
    UnexpectedResponse,
    ResendLimit,
    Busy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AckAction {
    Accepted,
    Retry,
    Unexpected,
}

#[derive(Clone, Copy)]
pub struct Status {
    pub controller: bool,
    pub config: u8,
    pub keyboard_port: bool,
    pub mouse_port: bool,
    pub irq_registered: bool,
    pub irq_enabled: bool,
    pub keyboard_dropped: u64,
    pub mouse_dropped: u64,
}

static CONTROLLER: AtomicBool = AtomicBool::new(false);
static KEYBOARD_PORT: AtomicBool = AtomicBool::new(false);
static MOUSE_PORT: AtomicBool = AtomicBool::new(false);
static IRQ_REGISTERED: AtomicBool = AtomicBool::new(false);
static IRQ_ENABLED: AtomicBool = AtomicBool::new(false);
static CONFIG: AtomicU8 = AtomicU8::new(0);
static COMMAND_LOCK: AtomicBool = AtomicBool::new(false);

static KEYBOARD_HEAD: AtomicUsize = AtomicUsize::new(0);
static KEYBOARD_TAIL: AtomicUsize = AtomicUsize::new(0);
static MOUSE_HEAD: AtomicUsize = AtomicUsize::new(0);
static MOUSE_TAIL: AtomicUsize = AtomicUsize::new(0);
static KEYBOARD_DROPPED: AtomicU64 = AtomicU64::new(0);
static MOUSE_DROPPED: AtomicU64 = AtomicU64::new(0);
static mut KEYBOARD_IRQ_ID: Option<crate::irq::RegistrationId> = None;
static mut MOUSE_IRQ_ID: Option<crate::irq::RegistrationId> = None;
static mut KEYBOARD_QUEUE: [u8; QUEUE_CAPACITY] = [0; QUEUE_CAPACITY];
static mut MOUSE_QUEUE: [u8; QUEUE_CAPACITY] = [0; QUEUE_CAPACITY];

pub fn contract_self_check() {
    assert_eq!(ack_action(ACK), AckAction::Accepted);
    assert_eq!(ack_action(RESEND), AckAction::Retry);
    assert_eq!(ack_action(0), AckAction::Unexpected);
    assert_eq!(next_index(QUEUE_CAPACITY - 1), 0);
    assert_eq!(next_index(0), 1);
}

pub fn init() -> InitResult {
    reset_state();
    let result = with_command(initialize);
    match result {
        Ok((config, keyboard, mouse)) => {
            CONFIG.store(config, Ordering::Release);
            KEYBOARD_PORT.store(keyboard, Ordering::Release);
            MOUSE_PORT.store(mouse, Ordering::Release);
            CONTROLLER.store(true, Ordering::Release);
            InitResult::Ready
        }
        Err(Error::ControllerSelfTest | Error::PortSelfTest) => InitResult::Failed,
        Err(Error::Timeout | Error::UnexpectedResponse | Error::ResendLimit | Error::Busy) => {
            InitResult::Unsupported
        }
    }
}

fn reset_state() {
    CONTROLLER.store(false, Ordering::Release);
    KEYBOARD_PORT.store(false, Ordering::Release);
    MOUSE_PORT.store(false, Ordering::Release);
    IRQ_REGISTERED.store(false, Ordering::Release);
    IRQ_ENABLED.store(false, Ordering::Release);
    CONFIG.store(0, Ordering::Release);
    KEYBOARD_HEAD.store(0, Ordering::Release);
    KEYBOARD_TAIL.store(0, Ordering::Release);
    MOUSE_HEAD.store(0, Ordering::Release);
    MOUSE_TAIL.store(0, Ordering::Release);
    KEYBOARD_DROPPED.store(0, Ordering::Relaxed);
    MOUSE_DROPPED.store(0, Ordering::Relaxed);
}

fn initialize() -> Result<(u8, bool, bool), Error> {
    let _ = region();
    controller_command(DISABLE_FIRST_PORT)?;
    controller_command(DISABLE_SECOND_PORT)?;
    flush_output()?;

    let mut config = read_config()?;
    config &= !(FIRST_IRQ | SECOND_IRQ | TRANSLATION);
    write_config(config)?;

    flush_output()?;
    controller_command(TEST_CONTROLLER)?;
    if read_data_after_output()? != CONTROLLER_OK {
        return Err(Error::ControllerSelfTest);
    }

    let keyboard = test_port(TEST_FIRST_PORT)?;
    let mouse = test_port(TEST_SECOND_PORT)?;
    if !keyboard && !mouse {
        return Err(Error::PortSelfTest);
    }
    if keyboard {
        controller_command(ENABLE_FIRST_PORT)?;
    }
    if mouse {
        controller_command(ENABLE_SECOND_PORT)?;
    }
    Ok((config, keyboard, mouse))
}

fn test_port(command: u8) -> Result<bool, Error> {
    flush_output()?;
    controller_command(command)?;
    Ok(read_data_after_output()? == PORT_OK)
}

pub fn status() -> Status {
    Status {
        controller: CONTROLLER.load(Ordering::Acquire),
        config: CONFIG.load(Ordering::Acquire),
        keyboard_port: KEYBOARD_PORT.load(Ordering::Acquire),
        mouse_port: MOUSE_PORT.load(Ordering::Acquire),
        irq_registered: IRQ_REGISTERED.load(Ordering::Acquire),
        irq_enabled: IRQ_ENABLED.load(Ordering::Acquire),
        keyboard_dropped: KEYBOARD_DROPPED.load(Ordering::Relaxed),
        mouse_dropped: MOUSE_DROPPED.load(Ordering::Relaxed),
    }
}

pub fn pop_keyboard() -> Option<u8> {
    dequeue(
        &KEYBOARD_HEAD,
        &KEYBOARD_TAIL,
        core::ptr::addr_of!(KEYBOARD_QUEUE),
    )
}

pub fn pop_mouse() -> Option<u8> {
    dequeue(&MOUSE_HEAD, &MOUSE_TAIL, core::ptr::addr_of!(MOUSE_QUEUE))
}

pub fn send_device_command(port: Port, command: u8) -> Result<(), &'static str> {
    if !port_available(port) {
        return Err("ps/2 port unavailable");
    }
    match with_command(|| send_device_command_locked(port, command)) {
        Ok(()) => Ok(()),
        Err(Error::Timeout) => Err("ps/2 command timeout"),
        Err(Error::ResendLimit) => Err("ps/2 command resend limit"),
        Err(Error::UnexpectedResponse) => Err("ps/2 unexpected response"),
        Err(Error::ControllerSelfTest | Error::PortSelfTest | Error::Busy) => {
            Err("ps/2 command unavailable")
        }
    }
}

pub fn send_device_data(port: Port, command: u8, data: u8) -> Result<(), &'static str> {
    if !port_available(port) {
        return Err("ps/2 port unavailable");
    }
    match with_command(|| {
        send_device_command_locked(port, command)?;
        send_device_command_locked(port, data)
    }) {
        Ok(()) => Ok(()),
        Err(Error::Timeout) => Err("ps/2 command timeout"),
        Err(Error::ResendLimit) => Err("ps/2 command resend limit"),
        Err(Error::UnexpectedResponse) => Err("ps/2 unexpected response"),
        Err(Error::ControllerSelfTest | Error::PortSelfTest | Error::Busy) => {
            Err("ps/2 command unavailable")
        }
    }
}

pub fn identify_device(port: Port) -> Result<u8, &'static str> {
    if !port_available(port) {
        return Err("ps/2 port unavailable");
    }
    match with_command(|| {
        send_device_command_locked(port, 0xf2)?;
        wait_for_device_byte(port)
    }) {
        Ok(byte) => Ok(byte),
        Err(Error::Timeout) => Err("ps/2 identify timeout"),
        Err(Error::ResendLimit) => Err("ps/2 identify resend limit"),
        Err(Error::UnexpectedResponse) => Err("ps/2 identify response error"),
        Err(Error::ControllerSelfTest | Error::PortSelfTest | Error::Busy) => {
            Err("ps/2 identify unavailable")
        }
    }
}

fn send_device_command_locked(port: Port, command: u8) -> Result<(), Error> {
    drain_to_queues();
    for _ in 0..COMMAND_RETRIES {
        if port.is_mouse() {
            controller_command(WRITE_SECOND_PORT)?;
        }
        write_data(command)?;
        match wait_for_ack(port)? {
            AckAction::Accepted => return Ok(()),
            AckAction::Retry => continue,
            AckAction::Unexpected => return Err(Error::UnexpectedResponse),
        }
    }
    Err(Error::ResendLimit)
}

fn wait_for_ack(port: Port) -> Result<AckAction, Error> {
    for _ in 0..POLL_LIMIT {
        let status = read_status();
        if status & OUTPUT_FULL == 0 {
            continue;
        }
        let byte = read_data()?;
        if status & AUX_OUTPUT != 0 && !port.is_mouse() {
            enqueue(Port::Mouse, byte);
            continue;
        }
        if status & AUX_OUTPUT == 0 && port.is_mouse() {
            enqueue(Port::Keyboard, byte);
            continue;
        }
        return Ok(ack_action(byte));
    }
    Err(Error::Timeout)
}

fn wait_for_device_byte(port: Port) -> Result<u8, Error> {
    for _ in 0..POLL_LIMIT {
        let status = read_status();
        if status & OUTPUT_FULL == 0 {
            continue;
        }
        let byte = read_data()?;
        if status & AUX_OUTPUT != 0 && !port.is_mouse() {
            enqueue(Port::Mouse, byte);
            continue;
        }
        if status & AUX_OUTPUT == 0 && port.is_mouse() {
            enqueue(Port::Keyboard, byte);
            continue;
        }
        return Ok(byte);
    }
    Err(Error::Timeout)
}

pub fn register_irqs() -> bool {
    if !CONTROLLER.load(Ordering::Acquire) {
        return true;
    }
    if IRQ_REGISTERED.load(Ordering::Acquire) {
        return true;
    }
    let mut keyboard_id = None;
    let mut mouse_id = None;
    if KEYBOARD_PORT.load(Ordering::Acquire) {
        let Ok(id) = crate::irq::register_owned(
            5,
            crate::drivers::framework::IrqKind::Legacy,
            1,
            33,
            keyboard_hard,
            None,
        ) else {
            return false;
        };
        keyboard_id = Some(id);
    }
    if MOUSE_PORT.load(Ordering::Acquire) {
        let Ok(id) = crate::irq::register_owned(
            6,
            crate::drivers::framework::IrqKind::Legacy,
            12,
            44,
            mouse_hard,
            None,
        ) else {
            if let Some(id) = keyboard_id {
                let _ = crate::irq::unregister(id);
            }
            return false;
        };
        mouse_id = Some(id);
    }
    unsafe {
        KEYBOARD_IRQ_ID = keyboard_id;
        MOUSE_IRQ_ID = mouse_id;
    }
    IRQ_REGISTERED.store(true, Ordering::Release);
    true
}

#[allow(dead_code)]
pub fn unregister_irqs() -> bool {
    if !IRQ_REGISTERED.load(Ordering::Acquire) {
        return true;
    }
    if IRQ_ENABLED.swap(false, Ordering::AcqRel) {
        if KEYBOARD_PORT.load(Ordering::Acquire) {
            crate::arch::disable_legacy_irq(1);
        }
        if MOUSE_PORT.load(Ordering::Acquire) {
            crate::arch::disable_legacy_irq(12);
        }
    }
    let result = unsafe {
        let keyboard = match core::ptr::read(core::ptr::addr_of!(KEYBOARD_IRQ_ID)) {
            Some(id) if crate::irq::unregister(id).is_ok() => {
                core::ptr::write(core::ptr::addr_of_mut!(KEYBOARD_IRQ_ID), None);
                true
            }
            Some(_) => false,
            None => true,
        };
        let mouse = match core::ptr::read(core::ptr::addr_of!(MOUSE_IRQ_ID)) {
            Some(id) if crate::irq::unregister(id).is_ok() => {
                core::ptr::write(core::ptr::addr_of_mut!(MOUSE_IRQ_ID), None);
                true
            }
            Some(_) => false,
            None => true,
        };
        keyboard && mouse
    };
    if result {
        IRQ_REGISTERED.store(false, Ordering::Release);
    }
    result
}

pub fn enable_interrupts() -> bool {
    if !CONTROLLER.load(Ordering::Acquire) {
        return true;
    }
    let result = with_command(|| {
        let mut config = read_config()?;
        config &= !(FIRST_IRQ | SECOND_IRQ);
        if KEYBOARD_PORT.load(Ordering::Acquire) {
            config |= FIRST_IRQ;
        }
        if MOUSE_PORT.load(Ordering::Acquire) {
            config |= SECOND_IRQ;
        }
        write_config(config)?;
        CONFIG.store(config, Ordering::Release);
        Ok(())
    });
    if result.is_err() {
        return false;
    }
    if KEYBOARD_PORT.load(Ordering::Acquire) {
        crate::arch::enable_legacy_irq(1);
    }
    if MOUSE_PORT.load(Ordering::Acquire) {
        crate::arch::enable_legacy_irq(12);
    }
    IRQ_ENABLED.store(true, Ordering::Release);
    true
}

fn keyboard_hard() -> bool {
    drain_irq(Port::Keyboard)
}

fn mouse_hard() -> bool {
    drain_irq(Port::Mouse)
}

fn drain_irq(port: Port) -> bool {
    if !port_available(port) {
        return false;
    }
    let mut received = false;
    for _ in 0..IRQ_DRAIN_LIMIT {
        let status = read_status();
        if status & OUTPUT_FULL == 0 {
            break;
        }
        let is_mouse = status & AUX_OUTPUT != 0;
        if is_mouse != port.is_mouse() {
            break;
        }
        let Ok(byte) = read_data() else {
            break;
        };
        enqueue(port, byte);
        received = true;
    }
    received
}

fn drain_to_queues() {
    for _ in 0..IRQ_DRAIN_LIMIT {
        let status = read_status();
        if status & OUTPUT_FULL == 0 {
            break;
        }
        let port = if status & AUX_OUTPUT != 0 {
            Port::Mouse
        } else {
            Port::Keyboard
        };
        let Ok(byte) = read_data() else {
            break;
        };
        if port_available(port) {
            enqueue(port, byte);
        }
    }
}

fn with_command<T>(f: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    if COMMAND_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return Err(Error::Busy);
    }
    let result = crate::arch::without_interrupts(f);
    COMMAND_LOCK.store(false, Ordering::Release);
    result
}

fn controller_command(command: u8) -> Result<(), Error> {
    wait_input_empty()?;
    write_status_command(command)
}

fn read_config() -> Result<u8, Error> {
    controller_command(READ_CONFIG)?;
    read_data_after_output()
}

fn write_config(config: u8) -> Result<(), Error> {
    controller_command(WRITE_CONFIG)?;
    write_data(config)
}

fn flush_output() -> Result<(), Error> {
    for _ in 0..POLL_LIMIT {
        if read_status() & OUTPUT_FULL == 0 {
            return Ok(());
        }
        let _ = read_data()?;
    }
    Err(Error::Timeout)
}

fn wait_input_empty() -> Result<(), Error> {
    for _ in 0..POLL_LIMIT {
        if read_status() & INPUT_FULL == 0 {
            return Ok(());
        }
    }
    Err(Error::Timeout)
}

fn read_data_after_output() -> Result<u8, Error> {
    for _ in 0..POLL_LIMIT {
        if read_status() & OUTPUT_FULL != 0 {
            return read_data();
        }
    }
    Err(Error::Timeout)
}

fn read_status() -> u8 {
    crate::arch::port_read(STATUS_COMMAND_PORT)
}

fn read_data() -> Result<u8, Error> {
    region().and_then(|io| io.read_u8(0)).ok_or(Error::Timeout)
}

fn write_status_command(command: u8) -> Result<(), Error> {
    region()
        .map(|io| io.write_u8(4, command))
        .filter(|ok| *ok)
        .ok_or(Error::Timeout)
        .map(|_| ())
}

fn write_data(data: u8) -> Result<(), Error> {
    region()
        .map(|io| io.write_u8(0, data))
        .filter(|ok| *ok)
        .ok_or(Error::Timeout)
        .map(|_| ())
}

fn region() -> Option<crate::io::PioRegion> {
    crate::io::PioRegion::new(DATA_PORT, 5)
}

fn ack_action(byte: u8) -> AckAction {
    match byte {
        ACK => AckAction::Accepted,
        RESEND => AckAction::Retry,
        _ => AckAction::Unexpected,
    }
}

fn port_available(port: Port) -> bool {
    match port {
        Port::Keyboard => KEYBOARD_PORT.load(Ordering::Acquire),
        Port::Mouse => MOUSE_PORT.load(Ordering::Acquire),
    }
}

fn next_index(index: usize) -> usize {
    if index + 1 == QUEUE_CAPACITY {
        0
    } else {
        index + 1
    }
}

fn enqueue(port: Port, byte: u8) {
    let (head, tail, queue, dropped) = match port {
        Port::Keyboard => (
            &KEYBOARD_HEAD,
            &KEYBOARD_TAIL,
            core::ptr::addr_of_mut!(KEYBOARD_QUEUE),
            &KEYBOARD_DROPPED,
        ),
        Port::Mouse => (
            &MOUSE_HEAD,
            &MOUSE_TAIL,
            core::ptr::addr_of_mut!(MOUSE_QUEUE),
            &MOUSE_DROPPED,
        ),
    };
    let current = head.load(Ordering::Relaxed);
    let next = next_index(current);
    if next == tail.load(Ordering::Acquire) {
        dropped.fetch_add(1, Ordering::Relaxed);
        return;
    }
    unsafe {
        (*queue)[current] = byte;
    }
    head.store(next, Ordering::Release);
}

fn dequeue(
    head: &AtomicUsize,
    tail: &AtomicUsize,
    queue: *const [u8; QUEUE_CAPACITY],
) -> Option<u8> {
    let current = tail.load(Ordering::Relaxed);
    if current == head.load(Ordering::Acquire) {
        return None;
    }
    let byte = unsafe { (*queue)[current] };
    tail.store(next_index(current), Ordering::Release);
    Some(byte)
}
