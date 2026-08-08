use crate::input::{Event, KeyCode, KeyEvent, Modifiers};

const SET_SCANCODE: u8 = 0xf0;
const SET_TWO: u8 = 0x02;
const ENABLE_SCANNING: u8 = 0xf4;
const POLL_LIMIT: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Us,
    Ru,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepeatPolicy {
    Disabled,
    Hardware,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InitResult {
    Ready,
    Unsupported,
    Failed,
}

#[derive(Clone, Copy)]
pub struct Status {
    pub layout: Layout,
    pub repeat: RepeatPolicy,
    pub held: usize,
    pub parser_pending: bool,
    pub events_dropped: u64,
}

#[derive(Clone, Copy, Debug)]
struct ScanEvent {
    code: KeyCode,
    pressed: bool,
}

#[derive(Clone, Copy)]
struct Parser {
    extended: bool,
    break_code: bool,
    pause_index: u8,
}

impl Parser {
    const fn new() -> Self {
        Self {
            extended: false,
            break_code: false,
            pause_index: 0,
        }
    }

    fn feed(&mut self, byte: u8) -> Option<ScanEvent> {
        const PAUSE: [u8; 8] = [0xe1, 0x14, 0x77, 0xe1, 0xf0, 0x14, 0xf0, 0x77];
        if self.pause_index != 0 {
            let index = self.pause_index as usize;
            if byte != PAUSE[index] {
                self.pause_index = 0;
                return None;
            }
            self.pause_index += 1;
            if self.pause_index == PAUSE.len() as u8 {
                self.pause_index = 0;
                return Some(ScanEvent {
                    code: KeyCode::Pause,
                    pressed: true,
                });
            }
            return None;
        }
        if byte == PAUSE[0] {
            self.pause_index = 1;
            self.extended = false;
            self.break_code = false;
            return None;
        }
        if byte == 0xe0 {
            self.extended = true;
            return None;
        }
        if byte == 0xf0 {
            self.break_code = true;
            return None;
        }
        let event = map_scan_code(self.extended, byte).map(|code| ScanEvent {
            code,
            pressed: !self.break_code,
        });
        self.extended = false;
        self.break_code = false;
        event
    }

    fn pending(self) -> bool {
        self.extended || self.break_code || self.pause_index != 0
    }
}

#[derive(Clone, Copy)]
struct KeyboardState {
    parser: Parser,
    modifiers: Modifiers,
    layout: Layout,
    repeat: RepeatPolicy,
    held: [bool; KeyCode::COUNT],
}

impl KeyboardState {
    const fn new() -> Self {
        Self {
            parser: Parser::new(),
            modifiers: Modifiers::empty(),
            layout: Layout::Us,
            repeat: RepeatPolicy::Hardware,
            held: [false; KeyCode::COUNT],
        }
    }

    fn feed(&mut self, byte: u8) -> Option<KeyEvent> {
        let ScanEvent { code, pressed } = self.parser.feed(byte)?;
        let index = code as usize;
        let was_held = self.held[index];
        if pressed {
            if was_held && (!is_repeatable(code) || self.repeat == RepeatPolicy::Disabled) {
                return None;
            }
            if !was_held {
                self.held[index] = true;
                self.apply_press(code);
            }
        } else {
            if !was_held {
                return None;
            }
            self.held[index] = false;
            self.apply_release(code);
        }
        let modifiers = self.modifiers;
        Some(KeyEvent {
            code,
            pressed,
            repeat: pressed && was_held,
            modifiers,
            text: if pressed {
                text_for(code, self.layout, modifiers)
            } else {
                None
            },
        })
    }

    fn apply_press(&mut self, code: KeyCode) {
        match code {
            KeyCode::LeftShift | KeyCode::RightShift => self.set_modifier(Modifiers::SHIFT, true),
            KeyCode::LeftCtrl | KeyCode::RightCtrl => self.set_modifier(Modifiers::CTRL, true),
            KeyCode::LeftAlt | KeyCode::RightAlt => self.set_modifier(Modifiers::ALT, true),
            KeyCode::LeftGui | KeyCode::RightGui => self.set_modifier(Modifiers::GUI, true),
            KeyCode::CapsLock => self.toggle_modifier(Modifiers::CAPS_LOCK),
            KeyCode::NumLock => self.toggle_modifier(Modifiers::NUM_LOCK),
            KeyCode::ScrollLock => self.toggle_modifier(Modifiers::SCROLL_LOCK),
            _ => {}
        }
    }

    fn apply_release(&mut self, code: KeyCode) {
        match code {
            KeyCode::LeftShift | KeyCode::RightShift => self.set_modifier(Modifiers::SHIFT, false),
            KeyCode::LeftCtrl | KeyCode::RightCtrl => self.set_modifier(Modifiers::CTRL, false),
            KeyCode::LeftAlt | KeyCode::RightAlt => self.set_modifier(Modifiers::ALT, false),
            KeyCode::LeftGui | KeyCode::RightGui => self.set_modifier(Modifiers::GUI, false),
            _ => {}
        }
    }

    fn set_modifier(&mut self, mask: u8, enabled: bool) {
        let mut bits = self.modifiers.bits();
        if enabled {
            bits |= mask;
        } else {
            bits &= !mask;
        }
        self.modifiers = Modifiers::from_bits(bits);
    }

    fn toggle_modifier(&mut self, mask: u8) {
        self.set_modifier(mask, !self.modifiers.contains(mask));
    }
}

static mut STATE: KeyboardState = KeyboardState::new();

pub fn contract_self_check() {
    let mut parser = Parser::new();
    let make = parser.feed(0x1c).expect("set-2 make");
    assert_eq!(make.code, KeyCode::A);
    assert!(make.pressed);
    assert!(parser.feed(0xf0).is_none());
    let break_event = parser.feed(0x1c).expect("set-2 break");
    assert_eq!(break_event.code, KeyCode::A);
    assert!(!break_event.pressed);
    assert!(parser.feed(0xe0).is_none());
    let arrow = parser.feed(0x75).expect("extended make");
    assert_eq!(arrow.code, KeyCode::ArrowUp);
    assert!(arrow.pressed);
    let mut pause = Parser::new();
    let mut pause_event = None;
    for byte in [0xe1, 0x14, 0x77, 0xe1, 0xf0, 0x14, 0xf0, 0x77] {
        if let Some(event) = pause.feed(byte) {
            pause_event = Some(event);
        }
    }
    assert_eq!(pause_event.map(|event| event.code), Some(KeyCode::Pause));
    let mut state = KeyboardState::new();
    let lower = state.feed(0x1c).expect("lowercase event");
    assert_eq!(lower.text, Some('a'));
    let _ = state.feed(0xf0);
    let _ = state.feed(0x1c);
    let _ = state.feed(0x12);
    let upper = state.feed(0x1c).expect("shifted event");
    assert_eq!(upper.text, Some('A'));
    let _ = state.feed(0xf0);
    let _ = state.feed(0x1c);
    let _ = state.feed(0xf0);
    let _ = state.feed(0x12);
    state.layout = Layout::Ru;
    let russian = state.feed(0x1c).expect("russian event");
    assert_eq!(russian.text, Some('ф'));
    state.repeat = RepeatPolicy::Disabled;
    assert!(state.feed(0x1c).is_none());
}

pub fn init() -> InitResult {
    crate::input::reset();
    with_state(|state| *state = KeyboardState::new());
    if !crate::drivers::ps2::status().keyboard_port {
        return InitResult::Unsupported;
    }
    if crate::drivers::ps2::send_device_command(crate::drivers::ps2::Port::Keyboard, SET_SCANCODE)
        .is_err()
        || crate::drivers::ps2::send_device_command(crate::drivers::ps2::Port::Keyboard, SET_TWO)
            .is_err()
        || crate::drivers::ps2::send_device_command(
            crate::drivers::ps2::Port::Keyboard,
            ENABLE_SCANNING,
        )
        .is_err()
    {
        return InitResult::Failed;
    }
    InitResult::Ready
}

pub fn poll() {
    for _ in 0..POLL_LIMIT {
        let Some(byte) = crate::drivers::ps2::pop_keyboard() else {
            break;
        };
        let event = with_state(|state| state.feed(byte));
        if let Some(event) = event {
            let _ = crate::input::push(Event::Key(event));
        }
    }
}

pub fn set_layout(layout: Layout) {
    with_state(|state| state.layout = layout);
}

pub fn set_repeat_policy(policy: RepeatPolicy) {
    with_state(|state| state.repeat = policy);
}

pub fn status() -> Status {
    with_state(|state| Status {
        layout: state.layout,
        repeat: state.repeat,
        held: state.held.iter().filter(|held| **held).count(),
        parser_pending: state.parser.pending(),
        events_dropped: crate::input::dropped(),
    })
}

fn with_state<R>(f: impl FnOnce(&mut KeyboardState) -> R) -> R {
    unsafe { f(&mut *core::ptr::addr_of_mut!(STATE)) }
}

fn is_repeatable(code: KeyCode) -> bool {
    !matches!(
        code,
        KeyCode::CapsLock
            | KeyCode::NumLock
            | KeyCode::ScrollLock
            | KeyCode::LeftShift
            | KeyCode::RightShift
            | KeyCode::LeftCtrl
            | KeyCode::RightCtrl
            | KeyCode::LeftAlt
            | KeyCode::RightAlt
            | KeyCode::LeftGui
            | KeyCode::RightGui
    )
}

fn text_for(code: KeyCode, layout: Layout, modifiers: Modifiers) -> Option<char> {
    let shifted = modifiers.contains(Modifiers::SHIFT);
    let caps = modifiers.contains(Modifiers::CAPS_LOCK);
    if let Some((lower, upper)) = letter_pair(code, layout) {
        return Some(if shifted ^ caps { upper } else { lower });
    }
    let value = match code {
        KeyCode::Num1 => {
            if shifted {
                '!'
            } else {
                '1'
            }
        }
        KeyCode::Num2 => {
            if shifted {
                '@'
            } else {
                '2'
            }
        }
        KeyCode::Num3 => {
            if shifted {
                '#'
            } else {
                '3'
            }
        }
        KeyCode::Num4 => {
            if shifted {
                '$'
            } else {
                '4'
            }
        }
        KeyCode::Num5 => {
            if shifted {
                '%'
            } else {
                '5'
            }
        }
        KeyCode::Num6 => {
            if shifted {
                '^'
            } else {
                '6'
            }
        }
        KeyCode::Num7 => {
            if shifted {
                '&'
            } else {
                '7'
            }
        }
        KeyCode::Num8 => {
            if shifted {
                '*'
            } else {
                '8'
            }
        }
        KeyCode::Num9 => {
            if shifted {
                '('
            } else {
                '9'
            }
        }
        KeyCode::Num0 => {
            if shifted {
                ')'
            } else {
                '0'
            }
        }
        KeyCode::Grave => {
            if shifted {
                '~'
            } else {
                '`'
            }
        }
        KeyCode::Minus => {
            if shifted {
                '_'
            } else {
                '-'
            }
        }
        KeyCode::Equals => {
            if shifted {
                '+'
            } else {
                '='
            }
        }
        KeyCode::LeftBracket => {
            if shifted {
                '{'
            } else {
                '['
            }
        }
        KeyCode::RightBracket => {
            if shifted {
                '}'
            } else {
                ']'
            }
        }
        KeyCode::Backslash => {
            if shifted {
                '|'
            } else {
                '\\'
            }
        }
        KeyCode::Semicolon => {
            if shifted {
                ':'
            } else {
                ';'
            }
        }
        KeyCode::Apostrophe => {
            if shifted {
                '"'
            } else {
                '\''
            }
        }
        KeyCode::Comma => {
            if shifted {
                '<'
            } else {
                ','
            }
        }
        KeyCode::Dot => {
            if shifted {
                '>'
            } else {
                '.'
            }
        }
        KeyCode::Slash => {
            if shifted {
                '?'
            } else {
                '/'
            }
        }
        KeyCode::Space => ' ',
        KeyCode::Tab => '\t',
        KeyCode::Enter => '\n',
        KeyCode::Backspace => '\x08',
        _ => return None,
    };
    Some(value)
}

fn letter_pair(code: KeyCode, layout: Layout) -> Option<(char, char)> {
    match layout {
        Layout::Us => match code {
            KeyCode::Q => Some(('q', 'Q')),
            KeyCode::W => Some(('w', 'W')),
            KeyCode::E => Some(('e', 'E')),
            KeyCode::R => Some(('r', 'R')),
            KeyCode::T => Some(('t', 'T')),
            KeyCode::Y => Some(('y', 'Y')),
            KeyCode::U => Some(('u', 'U')),
            KeyCode::I => Some(('i', 'I')),
            KeyCode::O => Some(('o', 'O')),
            KeyCode::P => Some(('p', 'P')),
            KeyCode::A => Some(('a', 'A')),
            KeyCode::S => Some(('s', 'S')),
            KeyCode::D => Some(('d', 'D')),
            KeyCode::F => Some(('f', 'F')),
            KeyCode::G => Some(('g', 'G')),
            KeyCode::H => Some(('h', 'H')),
            KeyCode::J => Some(('j', 'J')),
            KeyCode::K => Some(('k', 'K')),
            KeyCode::L => Some(('l', 'L')),
            KeyCode::Z => Some(('z', 'Z')),
            KeyCode::X => Some(('x', 'X')),
            KeyCode::C => Some(('c', 'C')),
            KeyCode::V => Some(('v', 'V')),
            KeyCode::B => Some(('b', 'B')),
            KeyCode::N => Some(('n', 'N')),
            KeyCode::M => Some(('m', 'M')),
            _ => None,
        },
        Layout::Ru => match code {
            KeyCode::Q => Some(('й', 'Й')),
            KeyCode::W => Some(('ц', 'Ц')),
            KeyCode::E => Some(('у', 'У')),
            KeyCode::R => Some(('к', 'К')),
            KeyCode::T => Some(('е', 'Е')),
            KeyCode::Y => Some(('н', 'Н')),
            KeyCode::U => Some(('г', 'Г')),
            KeyCode::I => Some(('ш', 'Ш')),
            KeyCode::O => Some(('щ', 'Щ')),
            KeyCode::P => Some(('з', 'З')),
            KeyCode::A => Some(('ф', 'Ф')),
            KeyCode::S => Some(('ы', 'Ы')),
            KeyCode::D => Some(('в', 'В')),
            KeyCode::F => Some(('а', 'А')),
            KeyCode::G => Some(('п', 'П')),
            KeyCode::H => Some(('р', 'Р')),
            KeyCode::J => Some(('о', 'О')),
            KeyCode::K => Some(('л', 'Л')),
            KeyCode::L => Some(('д', 'Д')),
            KeyCode::Z => Some(('я', 'Я')),
            KeyCode::X => Some(('ч', 'Ч')),
            KeyCode::C => Some(('с', 'С')),
            KeyCode::V => Some(('м', 'М')),
            KeyCode::B => Some(('и', 'И')),
            KeyCode::N => Some(('т', 'Т')),
            KeyCode::M => Some(('ь', 'Ь')),
            _ => None,
        },
    }
}

fn map_scan_code(extended: bool, byte: u8) -> Option<KeyCode> {
    if extended {
        return match byte {
            0x11 => Some(KeyCode::RightAlt),
            0x14 => Some(KeyCode::RightCtrl),
            0x1f => Some(KeyCode::LeftGui),
            0x27 => Some(KeyCode::RightGui),
            0x4a => Some(KeyCode::KeypadSlash),
            0x5a => Some(KeyCode::KeypadEnter),
            0x69 => Some(KeyCode::End),
            0x6b => Some(KeyCode::ArrowLeft),
            0x6c => Some(KeyCode::Home),
            0x70 => Some(KeyCode::Insert),
            0x71 => Some(KeyCode::Delete),
            0x72 => Some(KeyCode::ArrowDown),
            0x74 => Some(KeyCode::ArrowRight),
            0x75 => Some(KeyCode::ArrowUp),
            0x7a => Some(KeyCode::PageDown),
            0x7c => Some(KeyCode::PrintScreen),
            0x7d => Some(KeyCode::PageUp),
            _ => None,
        };
    }
    match byte {
        0x01 => Some(KeyCode::F9),
        0x03 => Some(KeyCode::F5),
        0x04 => Some(KeyCode::F3),
        0x05 => Some(KeyCode::F1),
        0x06 => Some(KeyCode::F2),
        0x07 => Some(KeyCode::F12),
        0x09 => Some(KeyCode::F10),
        0x0a => Some(KeyCode::F8),
        0x0b => Some(KeyCode::F6),
        0x0c => Some(KeyCode::F4),
        0x0d => Some(KeyCode::Tab),
        0x0e => Some(KeyCode::Grave),
        0x11 => Some(KeyCode::LeftAlt),
        0x12 => Some(KeyCode::LeftShift),
        0x14 => Some(KeyCode::LeftCtrl),
        0x15 => Some(KeyCode::Q),
        0x16 => Some(KeyCode::Num1),
        0x1a => Some(KeyCode::Z),
        0x1b => Some(KeyCode::S),
        0x1c => Some(KeyCode::A),
        0x1d => Some(KeyCode::W),
        0x1e => Some(KeyCode::Num2),
        0x21 => Some(KeyCode::C),
        0x22 => Some(KeyCode::X),
        0x23 => Some(KeyCode::D),
        0x24 => Some(KeyCode::E),
        0x25 => Some(KeyCode::Num4),
        0x26 => Some(KeyCode::Num3),
        0x29 => Some(KeyCode::Space),
        0x2a => Some(KeyCode::V),
        0x2b => Some(KeyCode::F),
        0x2c => Some(KeyCode::T),
        0x2d => Some(KeyCode::R),
        0x2e => Some(KeyCode::Num5),
        0x31 => Some(KeyCode::N),
        0x32 => Some(KeyCode::B),
        0x33 => Some(KeyCode::H),
        0x34 => Some(KeyCode::G),
        0x35 => Some(KeyCode::Y),
        0x36 => Some(KeyCode::Num6),
        0x3a => Some(KeyCode::M),
        0x3b => Some(KeyCode::J),
        0x3c => Some(KeyCode::U),
        0x3d => Some(KeyCode::Num7),
        0x3e => Some(KeyCode::Num8),
        0x41 => Some(KeyCode::Comma),
        0x42 => Some(KeyCode::K),
        0x43 => Some(KeyCode::I),
        0x44 => Some(KeyCode::O),
        0x45 => Some(KeyCode::Num0),
        0x46 => Some(KeyCode::Num9),
        0x49 => Some(KeyCode::Dot),
        0x4a => Some(KeyCode::Slash),
        0x4b => Some(KeyCode::L),
        0x4c => Some(KeyCode::Semicolon),
        0x4d => Some(KeyCode::P),
        0x4e => Some(KeyCode::Minus),
        0x52 => Some(KeyCode::Apostrophe),
        0x54 => Some(KeyCode::LeftBracket),
        0x55 => Some(KeyCode::Equals),
        0x58 => Some(KeyCode::CapsLock),
        0x59 => Some(KeyCode::RightShift),
        0x5a => Some(KeyCode::Enter),
        0x5b => Some(KeyCode::RightBracket),
        0x5d => Some(KeyCode::Backslash),
        0x66 => Some(KeyCode::Backspace),
        0x76 => Some(KeyCode::Escape),
        0x77 => Some(KeyCode::NumLock),
        0x7e => Some(KeyCode::ScrollLock),
        0x78 => Some(KeyCode::F11),
        0x83 => Some(KeyCode::F7),
        _ => None,
    }
}
