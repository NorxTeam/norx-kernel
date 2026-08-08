use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const EVENT_QUEUE_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Escape,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Grave,
    Num1,
    Num2,
    Num3,
    Num4,
    Num5,
    Num6,
    Num7,
    Num8,
    Num9,
    Num0,
    Minus,
    Equals,
    Backspace,
    Tab,
    Q,
    W,
    E,
    R,
    T,
    Y,
    U,
    I,
    O,
    P,
    LeftBracket,
    RightBracket,
    Backslash,
    CapsLock,
    A,
    S,
    D,
    F,
    G,
    H,
    J,
    K,
    L,
    Semicolon,
    Apostrophe,
    Enter,
    LeftShift,
    Z,
    X,
    C,
    V,
    B,
    N,
    M,
    Comma,
    Dot,
    Slash,
    RightShift,
    LeftCtrl,
    LeftAlt,
    Space,
    RightAlt,
    RightCtrl,
    LeftGui,
    RightGui,
    PrintScreen,
    ScrollLock,
    Pause,
    NumLock,
    KeypadSlash,
    KeypadEnter,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
}

impl KeyCode {
    pub const COUNT: usize = Self::ArrowRight as usize + 1;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const SHIFT: u8 = 1 << 0;
    pub const CTRL: u8 = 1 << 1;
    pub const ALT: u8 = 1 << 2;
    pub const GUI: u8 = 1 << 3;
    pub const CAPS_LOCK: u8 = 1 << 4;
    pub const NUM_LOCK: u8 = 1 << 5;
    pub const SCROLL_LOCK: u8 = 1 << 6;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn contains(self, mask: u8) -> bool {
        self.0 & mask != 0
    }
}

#[derive(Clone, Copy)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub pressed: bool,
    pub repeat: bool,
    pub modifiers: Modifiers,
    pub text: Option<char>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Buttons(u8);

impl Buttons {
    pub const LEFT: u8 = 1 << 0;
    pub const RIGHT: u8 = 1 << 1;
    pub const MIDDLE: u8 = 1 << 2;
    pub const BACK: u8 = 1 << 3;
    pub const FORWARD: u8 = 1 << 4;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy)]
pub struct PointerEvent {
    pub dx: i16,
    pub dy: i16,
    pub wheel: i8,
    pub buttons: Buttons,
    pub changed: Buttons,
}

#[derive(Clone, Copy)]
pub enum Event {
    Key(KeyEvent),
    Pointer(PointerEvent),
}

static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static mut EVENTS: [Option<Event>; EVENT_QUEUE_CAPACITY] = [None; EVENT_QUEUE_CAPACITY];

pub fn push(event: Event) -> bool {
    let head = HEAD.load(Ordering::Relaxed);
    let next = next_index(head);
    if next == TAIL.load(Ordering::Acquire) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    unsafe {
        let events = core::ptr::addr_of_mut!(EVENTS);
        (*events)[head] = Some(event);
    }
    HEAD.store(next, Ordering::Release);
    true
}

pub fn pop() -> Option<Event> {
    let tail = TAIL.load(Ordering::Relaxed);
    if tail == HEAD.load(Ordering::Acquire) {
        return None;
    }
    let event = unsafe {
        let events = core::ptr::addr_of!(EVENTS);
        (*events)[tail]
    };
    TAIL.store(next_index(tail), Ordering::Release);
    event
}

pub fn dropped() -> u64 {
    DROPPED.load(Ordering::Relaxed)
}

pub fn reset() {
    HEAD.store(0, Ordering::Release);
    TAIL.store(0, Ordering::Release);
    DROPPED.store(0, Ordering::Relaxed);
}

fn next_index(index: usize) -> usize {
    if index + 1 == EVENT_QUEUE_CAPACITY {
        0
    } else {
        index + 1
    }
}
