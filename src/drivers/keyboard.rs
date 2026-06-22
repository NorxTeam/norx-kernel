const CODE_CAP: usize = 64;

static mut SHIFT: bool = false;
static mut EXTENDED: bool = false;
static mut CODES: [u8; CODE_CAP] = [0; CODE_CAP];
static mut READ: usize = 0;
static mut WRITE: usize = 0;
static mut DROPPED: u64 = 0;

pub fn read() -> Option<crate::input::Key> {
    unsafe {
        if let Some(code) = pop_code() {
            return decode(code);
        }

        if crate::arch::inb(0x64) & 1 != 0 {
            return decode(crate::arch::inb(0x60));
        }
    }
    None
}

pub fn handle_interrupt() {
    unsafe {
        if crate::arch::inb(0x64) & 1 != 0 {
            push_code(crate::arch::inb(0x60));
        }
    }
}

pub fn pending() -> usize {
    unsafe { (WRITE + CODE_CAP - READ) % CODE_CAP }
}

pub fn dropped() -> u64 {
    unsafe { DROPPED }
}

unsafe fn push_code(code: u8) {
    let next = (WRITE + 1) % CODE_CAP;
    if next == READ {
        DROPPED = DROPPED.saturating_add(1);
        return;
    }
    CODES[WRITE] = code;
    WRITE = next;
}

unsafe fn pop_code() -> Option<u8> {
    crate::arch::without_interrupts(|| {
        if READ == WRITE {
            return None;
        }
        let code = CODES[READ];
        READ = (READ + 1) % CODE_CAP;
        Some(code)
    })
}

unsafe fn decode(code: u8) -> Option<crate::input::Key> {
    if code == 0xe0 {
        EXTENDED = true;
        return None;
    }

    let released = code & 0x80 != 0;
    let key = code & 0x7f;
    if key == 0x2a || key == 0x36 {
        SHIFT = !released;
        return None;
    }
    if released {
        EXTENDED = false;
        return None;
    }

    let byte = if EXTENDED {
        extended(key)
    } else {
        map(key, SHIFT)
    };
    EXTENDED = false;
    byte
}

fn extended(code: u8) -> Option<crate::input::Key> {
    match code {
        0x47 => Some(crate::input::Key::Home),
        0x48 => Some(crate::input::Key::Up),
        0x49 => Some(crate::input::Key::Home),
        0x4b => Some(crate::input::Key::Left),
        0x4d => Some(crate::input::Key::Right),
        0x4f => Some(crate::input::Key::End),
        0x50 => Some(crate::input::Key::Down),
        0x51 => Some(crate::input::Key::End),
        0x53 => Some(crate::input::Key::Delete),
        _ => None,
    }
}

fn map(code: u8, shift: bool) -> Option<crate::input::Key> {
    let b = match code {
        0x01 => return Some(crate::input::Key::Escape),
        0x02 => {
            if shift {
                b'!'
            } else {
                b'1'
            }
        }
        0x03 => {
            if shift {
                b'@'
            } else {
                b'2'
            }
        }
        0x04 => {
            if shift {
                b'#'
            } else {
                b'3'
            }
        }
        0x05 => {
            if shift {
                b'$'
            } else {
                b'4'
            }
        }
        0x06 => {
            if shift {
                b'%'
            } else {
                b'5'
            }
        }
        0x07 => {
            if shift {
                b'^'
            } else {
                b'6'
            }
        }
        0x08 => {
            if shift {
                b'&'
            } else {
                b'7'
            }
        }
        0x09 => {
            if shift {
                b'*'
            } else {
                b'8'
            }
        }
        0x0a => {
            if shift {
                b'('
            } else {
                b'9'
            }
        }
        0x0b => {
            if shift {
                b')'
            } else {
                b'0'
            }
        }
        0x0c => {
            if shift {
                b'_'
            } else {
                b'-'
            }
        }
        0x0d => {
            if shift {
                b'+'
            } else {
                b'='
            }
        }
        0x0e => 8,
        0x0f => b'\t',
        0x10 => letter(b'q', shift),
        0x11 => letter(b'w', shift),
        0x12 => letter(b'e', shift),
        0x13 => letter(b'r', shift),
        0x14 => letter(b't', shift),
        0x15 => letter(b'y', shift),
        0x16 => letter(b'u', shift),
        0x17 => letter(b'i', shift),
        0x18 => letter(b'o', shift),
        0x19 => letter(b'p', shift),
        0x1a => {
            if shift {
                b'{'
            } else {
                b'['
            }
        }
        0x1b => {
            if shift {
                b'}'
            } else {
                b']'
            }
        }
        0x1c => b'\n',
        0x1e => letter(b'a', shift),
        0x1f => letter(b's', shift),
        0x20 => letter(b'd', shift),
        0x21 => letter(b'f', shift),
        0x22 => letter(b'g', shift),
        0x23 => letter(b'h', shift),
        0x24 => letter(b'j', shift),
        0x25 => letter(b'k', shift),
        0x26 => letter(b'l', shift),
        0x27 => {
            if shift {
                b':'
            } else {
                b';'
            }
        }
        0x28 => {
            if shift {
                b'"'
            } else {
                b'\''
            }
        }
        0x29 => {
            if shift {
                b'~'
            } else {
                b'`'
            }
        }
        0x2b => {
            if shift {
                b'|'
            } else {
                b'\\'
            }
        }
        0x2c => letter(b'z', shift),
        0x2d => letter(b'x', shift),
        0x2e => letter(b'c', shift),
        0x2f => letter(b'v', shift),
        0x30 => letter(b'b', shift),
        0x31 => letter(b'n', shift),
        0x32 => letter(b'm', shift),
        0x33 => {
            if shift {
                b'<'
            } else {
                b','
            }
        }
        0x34 => {
            if shift {
                b'>'
            } else {
                b'.'
            }
        }
        0x35 => {
            if shift {
                b'?'
            } else {
                b'/'
            }
        }
        0x39 => b' ',
        _ => return None,
    };
    Some(match b {
        b'\n' => crate::input::Key::Enter,
        8 => crate::input::Key::Backspace,
        byte => crate::input::Key::Char(byte),
    })
}

fn letter(byte: u8, shift: bool) -> u8 {
    if shift {
        byte - 32
    } else {
        byte
    }
}
