use crate::input::{Buttons, Event, KeyCode, KeyEvent, Modifiers, PointerEvent};

const MAX_FIELDS: usize = 24;
const MAX_KEYS: usize = 6;
const MAX_REPORT_BYTES: usize = 64;
const MAX_DESCRIPTOR_BYTES: usize = 256;
const MAX_USAGES: usize = 8;
const NO_SELECTED_USAGE: u16 = u16::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyboard,
    Mouse,
    Other,
}

impl Kind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidItem,
    UnsupportedReportId,
    TooManyFields,
    InvalidLayout,
}

#[derive(Clone, Copy)]
struct Field {
    page: u16,
    usage_min: u16,
    usage_max: u16,
    usages: [u16; MAX_USAGES],
    usages_len: u8,
    selected_usage: u16,
    logical_min: i32,
    bit_offset: u16,
    bit_size: u8,
    count: u8,
}

impl Field {
    fn contains(self, page: u16, usage: u16) -> bool {
        self.page == page
            && if self.usages_len != 0 {
                self.usages[..self.usages_len as usize].contains(&usage)
            } else {
                (self.usage_min..=self.usage_max).contains(&usage)
            }
    }

    fn offset(self, usage: u16) -> Option<u16> {
        let usage = if self.selected_usage == NO_SELECTED_USAGE {
            usage
        } else {
            self.selected_usage
        };
        if !self.contains(self.page, usage) {
            return None;
        }
        let index = if self.usages_len != 0 {
            self.usages[..self.usages_len as usize]
                .iter()
                .position(|value| *value == usage)? as u16
        } else {
            usage.checked_sub(self.usage_min)?
        };
        self.bit_offset
            .checked_add(index.checked_mul(self.bit_size as u16)?)
    }

    fn for_usage(mut self, usage: u16) -> Self {
        self.selected_usage = usage;
        self
    }
}

#[derive(Clone, Copy)]
struct Layout {
    kind: Kind,
    report_bytes: usize,
    fields: [Option<Field>; MAX_FIELDS],
    fields_len: usize,
    modifiers: Option<Field>,
    keys: Option<Field>,
    buttons: Option<Field>,
    x: Option<Field>,
    y: Option<Field>,
    wheel: Option<Field>,
}

pub struct Device {
    layout: Layout,
    previous_keys: [u8; MAX_KEYS],
    previous_buttons: u8,
}

impl Device {
    pub fn from_descriptor(descriptor: &[u8]) -> Result<Self, Error> {
        if descriptor.is_empty() || descriptor.len() > MAX_DESCRIPTOR_BYTES {
            return Err(Error::InvalidItem);
        }
        let layout = parse_layout(descriptor)?;
        if layout.report_bytes == 0 || layout.report_bytes > MAX_REPORT_BYTES {
            return Err(Error::InvalidLayout);
        }
        Ok(Self {
            layout,
            previous_keys: [0; MAX_KEYS],
            previous_buttons: 0,
        })
    }

    pub const fn kind(&self) -> Kind {
        self.layout.kind
    }

    pub const fn report_bytes(&self) -> usize {
        self.layout.report_bytes
    }

    pub fn feed(&mut self, report: &[u8]) {
        if report.len() < self.layout.report_bytes {
            return;
        }
        match self.layout.kind {
            Kind::Keyboard => self.feed_keyboard(report),
            Kind::Mouse => self.feed_mouse(report),
            Kind::Other => {}
        }
    }

    fn feed_keyboard(&mut self, report: &[u8]) {
        let Some(modifier_field) = self.layout.modifiers else {
            return;
        };
        let Some(key_field) = self.layout.keys else {
            return;
        };
        let modifier_value = read_bits(
            report,
            modifier_field.bit_offset,
            modifier_field.bit_size as usize * modifier_field.count as usize,
        );
        let modifier_bits = hid_modifiers(modifier_value as u8);
        let mut current = [0u8; MAX_KEYS];
        let count = (key_field.count as usize).min(MAX_KEYS);
        for (index, key) in current.iter_mut().take(count).enumerate() {
            *key = read_field(report, key_field, key_field.usage_min + index as u16) as u8;
        }

        for usage in current {
            if usage != 0 && !self.previous_keys.contains(&usage) {
                if let Some(code) = key_code(usage) {
                    let _ = crate::input::push(Event::Key(KeyEvent {
                        code,
                        pressed: true,
                        repeat: false,
                        modifiers: Modifiers::from_bits(modifier_bits),
                        text: key_text(usage, modifier_bits),
                    }));
                }
            }
        }
        for usage in self.previous_keys {
            if usage != 0 && !current.contains(&usage) {
                if let Some(code) = key_code(usage) {
                    let _ = crate::input::push(Event::Key(KeyEvent {
                        code,
                        pressed: false,
                        repeat: false,
                        modifiers: Modifiers::from_bits(modifier_bits),
                        text: None,
                    }));
                }
            }
        }
        self.previous_keys = current;
    }

    fn feed_mouse(&mut self, report: &[u8]) {
        let Some(button_field) = self.layout.buttons else {
            return;
        };
        let Some(x_field) = self.layout.x else {
            return;
        };
        let Some(y_field) = self.layout.y else {
            return;
        };
        let buttons = read_bits(
            report,
            button_field.bit_offset,
            button_field.bit_size as usize * button_field.count as usize,
        ) as u8;
        let dx = read_signed_field(report, x_field) as i16;
        let dy = read_signed_field(report, y_field) as i16;
        let wheel = self
            .layout
            .wheel
            .map(|field| read_signed_field(report, field) as i8)
            .unwrap_or(0);
        let changed = buttons ^ self.previous_buttons;
        self.previous_buttons = buttons;
        let _ = crate::input::push(Event::Pointer(PointerEvent {
            dx,
            dy,
            wheel,
            buttons: Buttons::from_bits(buttons & 0x1f),
            changed: Buttons::from_bits(changed & 0x1f),
        }));
    }
}

fn parse_layout(descriptor: &[u8]) -> Result<Layout, Error> {
    let mut layout = Layout {
        kind: Kind::Other,
        report_bytes: 0,
        fields: [None; MAX_FIELDS],
        fields_len: 0,
        modifiers: None,
        keys: None,
        buttons: None,
        x: None,
        y: None,
        wheel: None,
    };
    let mut offset = 0;
    let mut bit_offset: u16 = 0;
    let mut usage_page: u16 = 0;
    let mut report_size: u8 = 0;
    let mut report_count: u8 = 0;
    let mut logical_min: i32 = 0;
    let mut logical_max: i32 = 0;
    let mut usage_min: u16 = 0;
    let mut usage_max: u16 = 0;
    let mut usages = [0u16; MAX_USAGES];
    let mut usages_len = 0;
    let mut usage_set = false;
    while offset < descriptor.len() {
        let prefix = descriptor[offset];
        offset += 1;
        if prefix == 0xfe {
            if offset + 2 > descriptor.len() {
                return Err(Error::InvalidItem);
            }
            let length = descriptor[offset] as usize;
            offset = offset.checked_add(2 + length).ok_or(Error::InvalidItem)?;
            if offset > descriptor.len() {
                return Err(Error::InvalidItem);
            }
            continue;
        }
        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        if offset + size > descriptor.len() {
            return Err(Error::InvalidItem);
        }
        let value = item_value(&descriptor[offset..offset + size]);
        offset += size;
        let item_type = (prefix >> 2) & 0x03;
        let tag = prefix >> 4;
        match (item_type, tag) {
            (1, 0) => usage_page = value as u16,
            (1, 1) => logical_min = signed_value(value, size),
            (1, 2) => logical_max = signed_value(value, size),
            (1, 7) => report_size = value as u8,
            (1, 8) => {
                if value != 0 {
                    return Err(Error::UnsupportedReportId);
                }
            }
            (1, 9) => report_count = value as u8,
            (2, 1) => {
                usage_min = value as u16;
                usage_max = usage_min;
                usages_len = 0;
                usage_set = true;
            }
            (2, 2) => {
                usage_max = value as u16;
                usage_set = true;
            }
            (2, 0) => {
                if usages_len == MAX_USAGES {
                    return Err(Error::InvalidLayout);
                }
                usages[usages_len] = value as u16;
                usages_len += 1;
                if usages_len == 1 {
                    usage_min = value as u16;
                }
                usage_max = value as u16;
                usage_set = true;
            }
            (0, 8) => {
                if report_size == 0 || report_size > 32 || report_count == 0 || report_count > 32 {
                    return Err(Error::InvalidLayout);
                }
                let field = Field {
                    page: usage_page,
                    usage_min: if usage_set { usage_min } else { 0 },
                    usage_max: if usage_set {
                        usage_max.max(usage_min)
                    } else {
                        0
                    },
                    usages,
                    usages_len: usages_len as u8,
                    selected_usage: NO_SELECTED_USAGE,
                    logical_min,
                    bit_offset,
                    bit_size: report_size,
                    count: report_count,
                };
                if layout.fields_len == MAX_FIELDS {
                    return Err(Error::TooManyFields);
                }
                layout.fields[layout.fields_len] = Some(field);
                layout.fields_len += 1;
                classify_field(&mut layout, field);
                let bits = (report_size as u16)
                    .checked_mul(report_count as u16)
                    .ok_or(Error::InvalidLayout)?;
                bit_offset = bit_offset.checked_add(bits).ok_or(Error::InvalidLayout)?;
                usage_set = false;
                usages = [0; MAX_USAGES];
                usages_len = 0;
            }
            (0, 10) => {}
            _ => {}
        }
        let _ = (logical_min, logical_max);
    }
    layout.report_bytes = (bit_offset as usize).div_ceil(8);
    if layout.modifiers.is_some() && layout.keys.is_some() {
        layout.kind = Kind::Keyboard;
    } else if layout.buttons.is_some() && layout.x.is_some() && layout.y.is_some() {
        layout.kind = Kind::Mouse;
    }
    Ok(layout)
}

fn classify_field(layout: &mut Layout, field: Field) {
    if field.page == 0x07 && field.bit_size == 1 && field.count >= 8 {
        layout.modifiers = Some(field);
    } else if field.page == 0x07 && field.bit_size >= 8 && field.count >= 3 {
        layout.keys = Some(field);
    } else if field.page == 0x09 && field.bit_size == 1 {
        layout.buttons = Some(field);
    } else if field.page == 0x01 {
        if field.contains(field.page, 0x30) {
            layout.x = Some(field.for_usage(0x30));
        }
        if field.contains(field.page, 0x31) {
            layout.y = Some(field.for_usage(0x31));
        }
        if field.contains(field.page, 0x38) {
            layout.wheel = Some(field.for_usage(0x38));
        }
    }
}

fn item_value(bytes: &[u8]) -> u32 {
    let mut value = 0;
    for (index, byte) in bytes.iter().enumerate() {
        value |= (*byte as u32) << (index * 8);
    }
    value
}

fn signed_value(value: u32, size: usize) -> i32 {
    signed_bits(value, size * 8)
}

fn signed_bits(value: u32, bits: usize) -> i32 {
    if bits == 0 || bits >= 32 || value & (1 << (bits - 1)) == 0 {
        return value as i32;
    }
    (value | (!0u32 << bits)) as i32
}

fn read_field(report: &[u8], field: Field, usage: u16) -> u32 {
    let Some(offset) = field.offset(usage) else {
        return 0;
    };
    read_bits(report, offset, field.bit_size as usize)
}

fn read_bits(report: &[u8], bit_offset: u16, bit_count: usize) -> u32 {
    let mut value = 0;
    for bit in 0..bit_count.min(32) {
        let position = bit_offset as usize + bit;
        if report
            .get(position / 8)
            .is_some_and(|byte| byte & (1 << (position & 7)) != 0)
        {
            value |= 1u32 << bit;
        }
    }
    value
}

fn read_signed_field(report: &[u8], field: Field) -> i32 {
    let value = read_field(report, field, field.usage_min);
    if field.logical_min < 0 {
        signed_bits(value, field.bit_size as usize)
    } else {
        value as i32
    }
}

fn hid_modifiers(value: u8) -> u8 {
    let mut modifiers = 0;
    if value & 0x03 != 0 {
        modifiers |= Modifiers::SHIFT;
    }
    if value & 0x0c != 0 {
        modifiers |= Modifiers::CTRL;
    }
    if value & 0x30 != 0 {
        modifiers |= Modifiers::ALT;
    }
    if value & 0xc0 != 0 {
        modifiers |= Modifiers::GUI;
    }
    modifiers
}

fn key_code(usage: u8) -> Option<KeyCode> {
    Some(match usage {
        0x04 => KeyCode::A,
        0x05 => KeyCode::B,
        0x06 => KeyCode::C,
        0x07 => KeyCode::D,
        0x08 => KeyCode::E,
        0x09 => KeyCode::F,
        0x0a => KeyCode::G,
        0x0b => KeyCode::H,
        0x0c => KeyCode::I,
        0x0d => KeyCode::J,
        0x0e => KeyCode::K,
        0x0f => KeyCode::L,
        0x10 => KeyCode::M,
        0x11 => KeyCode::N,
        0x12 => KeyCode::O,
        0x13 => KeyCode::P,
        0x14 => KeyCode::Q,
        0x15 => KeyCode::R,
        0x16 => KeyCode::S,
        0x17 => KeyCode::T,
        0x18 => KeyCode::U,
        0x19 => KeyCode::V,
        0x1a => KeyCode::W,
        0x1b => KeyCode::X,
        0x1c => KeyCode::Y,
        0x1d => KeyCode::Z,
        0x1e => KeyCode::Num1,
        0x1f => KeyCode::Num2,
        0x20 => KeyCode::Num3,
        0x21 => KeyCode::Num4,
        0x22 => KeyCode::Num5,
        0x23 => KeyCode::Num6,
        0x24 => KeyCode::Num7,
        0x25 => KeyCode::Num8,
        0x26 => KeyCode::Num9,
        0x27 => KeyCode::Num0,
        0x28 => KeyCode::Enter,
        0x29 => KeyCode::Escape,
        0x2a => KeyCode::Backspace,
        0x2b => KeyCode::Tab,
        0x2c => KeyCode::Space,
        0x2d => KeyCode::Minus,
        0x2e => KeyCode::Equals,
        0x2f => KeyCode::LeftBracket,
        0x30 => KeyCode::RightBracket,
        0x31 => KeyCode::Backslash,
        0x33 => KeyCode::Semicolon,
        0x34 => KeyCode::Apostrophe,
        0x35 => KeyCode::Grave,
        0x36 => KeyCode::Comma,
        0x37 => KeyCode::Dot,
        0x38 => KeyCode::Slash,
        0x39 => KeyCode::CapsLock,
        0x3a..=0x45 => [
            KeyCode::F1,
            KeyCode::F2,
            KeyCode::F3,
            KeyCode::F4,
            KeyCode::F5,
            KeyCode::F6,
            KeyCode::F7,
            KeyCode::F8,
            KeyCode::F9,
            KeyCode::F10,
            KeyCode::F11,
            KeyCode::F12,
        ][(usage - 0x3a) as usize],
        0x47 => KeyCode::ScrollLock,
        0x48 => KeyCode::Pause,
        0x49 => KeyCode::Insert,
        0x4a => KeyCode::Home,
        0x4b => KeyCode::PageUp,
        0x4c => KeyCode::Delete,
        0x4d => KeyCode::End,
        0x4e => KeyCode::PageDown,
        0x4f => KeyCode::ArrowRight,
        0x50 => KeyCode::ArrowLeft,
        0x51 => KeyCode::ArrowDown,
        0x52 => KeyCode::ArrowUp,
        0x53 => KeyCode::NumLock,
        0xe0 => KeyCode::LeftCtrl,
        0xe1 => KeyCode::LeftShift,
        0xe2 => KeyCode::LeftAlt,
        0xe3 => KeyCode::LeftGui,
        0xe4 => KeyCode::RightCtrl,
        0xe5 => KeyCode::RightShift,
        0xe6 => KeyCode::RightAlt,
        0xe7 => KeyCode::RightGui,
        _ => return None,
    })
}

fn key_text(usage: u8, modifiers: u8) -> Option<char> {
    let shift = modifiers & Modifiers::SHIFT != 0;
    let letter = match usage {
        0x04..=0x1d => (b'a' + usage - 0x04) as char,
        0x1e => '1',
        0x1f => '2',
        0x20 => '3',
        0x21 => '4',
        0x22 => '5',
        0x23 => '6',
        0x24 => '7',
        0x25 => '8',
        0x26 => '9',
        0x27 => '0',
        0x2c => ' ',
        0x2d => '-',
        0x2e => '=',
        0x2f => '[',
        0x30 => ']',
        0x31 => '\\',
        0x33 => ';',
        0x34 => '\'',
        0x35 => '`',
        0x36 => ',',
        0x37 => '.',
        0x38 => '/',
        _ => return None,
    };
    Some(if shift {
        match letter {
            '1' => '!',
            '2' => '@',
            '3' => '#',
            '4' => '$',
            '5' => '%',
            '6' => '^',
            '7' => '&',
            '8' => '*',
            '9' => '(',
            '0' => ')',
            '-' => '_',
            '=' => '+',
            '[' => '{',
            ']' => '}',
            '\\' => '|',
            ';' => ':',
            '\'' => '"',
            '`' => '~',
            ',' => '<',
            '.' => '>',
            '/' => '?',
            value => value.to_ascii_uppercase(),
        }
    } else {
        letter
    })
}

pub fn contract_self_check() {
    const KEYBOARD: [u8; 63] = [
        0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25,
        0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x05,
        0x75, 0x01, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x91, 0x02, 0x95, 0x01, 0x75, 0x03, 0x91,
        0x01, 0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x25, 0x65, 0x05, 0x07, 0x19, 0x00, 0x29, 0x65,
        0x81, 0x00, 0xc0,
    ];
    let mut device = Device::from_descriptor(&KEYBOARD).unwrap();
    assert_eq!(device.kind(), Kind::Keyboard);
    assert_eq!(device.report_bytes(), 8);
    crate::input::reset();
    let mut report = [0; 8];
    report[0] = 2;
    report[2] = 4;
    device.feed(&report);
    assert!(
        matches!(crate::input::pop(), Some(Event::Key(event)) if event.pressed && event.modifiers.contains(Modifiers::SHIFT))
    );
    report[0] = 0;
    report[2] = 0;
    device.feed(&report);
    assert!(matches!(crate::input::pop(), Some(Event::Key(event)) if !event.pressed));

    const MOUSE: [u8; 52] = [
        0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x09, 0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29,
        0x03, 0x15, 0x00, 0x25, 0x01, 0x95, 0x03, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x05,
        0x81, 0x01, 0x05, 0x01, 0x09, 0x30, 0x09, 0x31, 0x09, 0x38, 0x15, 0x81, 0x25, 0x7f, 0x75,
        0x08, 0x95, 0x03, 0x81, 0x06, 0xc0, 0xc0,
    ];
    let mut mouse = Device::from_descriptor(&MOUSE).unwrap();
    assert_eq!(mouse.kind(), Kind::Mouse);
    assert_eq!(mouse.report_bytes(), 4);
    assert_eq!(mouse.layout.y.unwrap().logical_min, -127);
    assert_eq!(mouse.layout.y.unwrap().bit_size, 8);
    assert_eq!(signed_value(255, 1), -1);
    assert_eq!(signed_bits(255, 8), -1);
    assert_eq!(
        read_signed_field(&[1, 1, 0xff, 1], mouse.layout.y.unwrap()),
        -1
    );
    crate::input::reset();
    mouse.feed(&[1, 1, 0xff, 1]);
    let Some(Event::Pointer(event)) = crate::input::pop() else {
        panic!("HID mouse did not produce a pointer event");
    };
    assert_eq!(event.dx, 1);
    assert_eq!(event.dy, -1);
    assert_eq!(event.wheel, 1);
    assert_eq!(event.buttons.bits(), Buttons::LEFT);
    assert_eq!(event.changed.bits(), Buttons::LEFT);
}
