use crate::input::{Buttons, Event, PointerEvent};

const RESET_DEFAULTS: u8 = 0xf6;
const ENABLE_REPORTING: u8 = 0xf4;
const SET_SAMPLE_RATE: u8 = 0xf3;
const POLL_LIMIT: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InitResult {
    Ready,
    Unsupported,
    Failed,
}

#[derive(Clone, Copy)]
pub struct Status {
    pub id: u8,
    pub wheel: bool,
    pub extra_buttons: bool,
    pub packet_len: usize,
    pub parser_pending: bool,
    pub buttons: Buttons,
    pub events_dropped: u64,
}

#[derive(Clone, Copy)]
struct Capabilities {
    id: u8,
    wheel: bool,
    extra_buttons: bool,
}

impl Capabilities {
    const fn standard() -> Self {
        Self {
            id: 0,
            wheel: false,
            extra_buttons: false,
        }
    }

    const fn packet_len(self) -> usize {
        if self.wheel {
            4
        } else {
            3
        }
    }
}

#[derive(Clone, Copy)]
struct PacketParser {
    bytes: [u8; 4],
    len: usize,
    capabilities: Capabilities,
    buttons: Buttons,
}

impl PacketParser {
    const fn new() -> Self {
        Self {
            bytes: [0; 4],
            len: 0,
            capabilities: Capabilities::standard(),
            buttons: Buttons::empty(),
        }
    }

    fn configure(&mut self, capabilities: Capabilities) {
        self.capabilities = capabilities;
        self.len = 0;
        self.buttons = Buttons::empty();
    }

    fn feed(&mut self, byte: u8) -> Option<PointerEvent> {
        if self.len == 0 {
            if byte & 0x08 == 0 {
                return None;
            }
            self.bytes[0] = byte;
            self.len = 1;
            return None;
        }
        self.bytes[self.len] = byte;
        self.len += 1;
        if self.len != self.capabilities.packet_len() {
            return None;
        }
        let event = self.decode();
        self.len = 0;
        event
    }

    fn decode(&mut self) -> Option<PointerEvent> {
        let status = self.bytes[0];
        if status & 0xc0 != 0 {
            return None;
        }
        let dx = signed_byte(self.bytes[1], status & (1 << 4) != 0);
        let dy = -signed_byte(self.bytes[2], status & (1 << 5) != 0);
        let mut button_bits = status & (Buttons::LEFT | Buttons::RIGHT | Buttons::MIDDLE);
        let mut wheel = 0;
        if self.capabilities.wheel {
            let wheel_byte = self.bytes[3];
            wheel = signed_nibble(wheel_byte & 0x0f);
            if self.capabilities.extra_buttons {
                if wheel_byte & (1 << 4) != 0 {
                    button_bits |= Buttons::BACK;
                }
                if wheel_byte & (1 << 5) != 0 {
                    button_bits |= Buttons::FORWARD;
                }
            }
        }
        let buttons = Buttons::from_bits(button_bits);
        let changed = Buttons::from_bits(buttons.bits() ^ self.buttons.bits());
        self.buttons = buttons;
        Some(PointerEvent {
            dx,
            dy,
            wheel,
            buttons,
            changed,
        })
    }

    fn pending(self) -> bool {
        self.len != 0
    }
}

#[derive(Clone, Copy)]
struct MouseState {
    parser: PacketParser,
    capabilities: Capabilities,
}

impl MouseState {
    const fn new() -> Self {
        Self {
            parser: PacketParser::new(),
            capabilities: Capabilities::standard(),
        }
    }

    fn configure(&mut self, capabilities: Capabilities) {
        self.capabilities = capabilities;
        self.parser.configure(capabilities);
    }
}

static mut STATE: MouseState = MouseState::new();

pub fn contract_self_check() {
    let mut parser = PacketParser::new();
    assert!(parser.feed(0x00).is_none());
    let first = parser.feed(0x29);
    assert!(first.is_none());
    assert!(parser.feed(1).is_none());
    let event = parser.feed(0xff).expect("standard mouse packet");
    assert_eq!(event.dx, 1);
    assert_eq!(event.dy, 1);
    assert_eq!(event.buttons.bits(), Buttons::LEFT);
    assert_eq!(event.changed.bits(), Buttons::LEFT);

    let capabilities = Capabilities {
        id: 4,
        wheel: true,
        extra_buttons: true,
    };
    parser.configure(capabilities);
    assert!(parser.feed(0x08).is_none());
    assert!(parser.feed(0).is_none());
    assert!(parser.feed(0).is_none());
    let event = parser.feed(0x31).expect("wheel mouse packet");
    assert_eq!(event.wheel, 1);
    assert_eq!(event.buttons.bits() & Buttons::BACK, Buttons::BACK);
    assert_eq!(event.buttons.bits() & Buttons::FORWARD, Buttons::FORWARD);
}

pub fn init() -> InitResult {
    if !crate::drivers::ps2::status().mouse_port {
        return InitResult::Unsupported;
    }
    if crate::drivers::ps2::send_device_command(crate::drivers::ps2::Port::Mouse, RESET_DEFAULTS)
        .is_err()
    {
        return InitResult::Failed;
    }
    let mut id = match crate::drivers::ps2::identify_device(crate::drivers::ps2::Port::Mouse) {
        Ok(id) => id,
        Err(_) => return InitResult::Failed,
    };
    if id == 0 {
        for rate in [200, 100, 80] {
            if crate::drivers::ps2::send_device_data(
                crate::drivers::ps2::Port::Mouse,
                SET_SAMPLE_RATE,
                rate,
            )
            .is_err()
            {
                return InitResult::Failed;
            }
        }
        id = match crate::drivers::ps2::identify_device(crate::drivers::ps2::Port::Mouse) {
            Ok(id) => id,
            Err(_) => return InitResult::Failed,
        };
    }
    let mut extra_buttons = id == 4;
    let wheel = id == 3 || id == 4;
    if id == 3 {
        for rate in [200, 200, 80] {
            if crate::drivers::ps2::send_device_data(
                crate::drivers::ps2::Port::Mouse,
                SET_SAMPLE_RATE,
                rate,
            )
            .is_err()
            {
                return InitResult::Failed;
            }
        }
        if let Ok(extra_id) = crate::drivers::ps2::identify_device(crate::drivers::ps2::Port::Mouse)
        {
            id = extra_id;
            extra_buttons = id == 4;
        }
    }
    if crate::drivers::ps2::send_device_command(crate::drivers::ps2::Port::Mouse, ENABLE_REPORTING)
        .is_err()
    {
        return InitResult::Failed;
    }
    let capabilities = Capabilities {
        id,
        wheel,
        extra_buttons,
    };
    with_state(|state| state.configure(capabilities));
    InitResult::Ready
}

pub fn poll() {
    for _ in 0..POLL_LIMIT {
        let Some(byte) = crate::drivers::ps2::pop_mouse() else {
            break;
        };
        let event = with_state(|state| state.parser.feed(byte));
        if let Some(event) = event {
            let _ = crate::input::push(Event::Pointer(event));
        }
    }
}

pub fn status() -> Status {
    with_state(|state| Status {
        id: state.capabilities.id,
        wheel: state.capabilities.wheel,
        extra_buttons: state.capabilities.extra_buttons,
        packet_len: state.capabilities.packet_len(),
        parser_pending: state.parser.pending(),
        buttons: state.parser.buttons,
        events_dropped: crate::input::dropped(),
    })
}

fn with_state<R>(f: impl FnOnce(&mut MouseState) -> R) -> R {
    unsafe { f(&mut *core::ptr::addr_of_mut!(STATE)) }
}

fn signed_byte(byte: u8, negative: bool) -> i16 {
    if negative {
        (byte as i16) - 256
    } else {
        byte as i16
    }
}

fn signed_nibble(nibble: u8) -> i8 {
    if nibble & 0x08 != 0 {
        (nibble | 0xf0) as i8
    } else {
        nibble as i8
    }
}
