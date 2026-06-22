#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(u8),
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Escape,
}

static mut SERIAL_ESC: [u8; 4] = [0; 4];
static mut SERIAL_ESC_LEN: usize = 0;

pub fn init(_system_table: *mut crate::uefi::SystemTable) {
    crate::drivers::framework::register(crate::drivers::framework::Driver {
        name: "vm-keyboard",
        class: crate::drivers::framework::Class::Input,
        state: crate::drivers::framework::State::Ready,
    });
    crate::drivers::framework::register(crate::drivers::framework::Driver {
        name: "serial-shell",
        class: crate::drivers::framework::Class::Input,
        state: crate::drivers::framework::State::Ready,
    });
}

pub fn poll_user() -> Option<Key> {
    #[cfg(target_arch = "x86_64")]
    {
        crate::drivers::keyboard::read()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

pub fn poll_serial() -> Option<Key> {
    let byte = crate::drivers::serial::read()?;
    unsafe {
        if SERIAL_ESC_LEN != 0 || byte == 0x1b {
            SERIAL_ESC[SERIAL_ESC_LEN] = byte;
            SERIAL_ESC_LEN += 1;
            if SERIAL_ESC_LEN == 1 {
                return None;
            }
            if SERIAL_ESC_LEN == 2 && SERIAL_ESC[1] != b'[' {
                SERIAL_ESC_LEN = 0;
                return Some(Key::Escape);
            }
            if SERIAL_ESC_LEN < 3 {
                return None;
            }
            if SERIAL_ESC_LEN == 3 && SERIAL_ESC[2].is_ascii_digit() {
                return None;
            }
            let key = match SERIAL_ESC[2] {
                b'A' => Key::Up,
                b'B' => Key::Down,
                b'C' => Key::Right,
                b'D' => Key::Left,
                b'H' => Key::Home,
                b'F' => Key::End,
                b'3' if SERIAL_ESC_LEN == 4 && SERIAL_ESC[3] == b'~' => Key::Delete,
                _ => Key::Escape,
            };
            SERIAL_ESC_LEN = 0;
            return Some(key);
        }
    }

    match byte {
        b'\n' | b'\r' => Some(Key::Enter),
        8 | 127 => Some(Key::Backspace),
        0x01 => Some(Key::Home),
        0x05 => Some(Key::End),
        0x04 => Some(Key::Delete),
        0x1b => Some(Key::Escape),
        b'\t' => Some(Key::Char(b'\t')),
        0x20..=0x7e => Some(Key::Char(byte)),
        _ => None,
    }
}
