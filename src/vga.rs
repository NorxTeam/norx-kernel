use core::fmt::{self, Write};

const WIDTH: usize = 80;
const HEIGHT: usize = 25;
const MEMORY: *mut u8 = 0xb8000 as *mut u8;
const ATTRIBUTE: u8 = 0x07;

struct Vga {
    col: usize,
    row: usize,
    ansi: u8,
}

static mut STATE: Option<Vga> = None;

pub fn init() {
    clear();
    unsafe {
        STATE = Some(Vga {
            col: 0,
            row: 0,
            ansi: 0,
        });
    }
}

pub fn write(args: fmt::Arguments) {
    let _ = Writer.write_fmt(args);
}

struct Writer;

impl Write for Writer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for value in text.bytes() {
            byte(value);
        }
        Ok(())
    }
}

fn byte(value: u8) {
    unsafe {
        let Some(state) = (&raw mut STATE).as_mut().and_then(Option::as_mut) else {
            return;
        };

        if state.ansi != 0 {
            if state.ansi == 1 {
                state.ansi = if value == b'[' { 2 } else { 0 };
            } else if (b'@'..=b'~').contains(&value) {
                state.ansi = 0;
            }
            return;
        }

        match value {
            0x1b => state.ansi = 1,
            b'\n' => next_line(state),
            b'\r' => state.col = 0,
            8 => state.col = state.col.saturating_sub(1),
            0x20..=0x7e => put(state, value),
            _ => {}
        }
    }
}

fn put(state: &mut Vga, value: u8) {
    if state.col >= WIDTH {
        next_line(state);
    }
    if state.row >= HEIGHT {
        clear();
        state.col = 0;
        state.row = 0;
    }

    let offset = (state.row * WIDTH + state.col) * 2;
    unsafe {
        core::ptr::write_volatile(MEMORY.add(offset), value);
        core::ptr::write_volatile(MEMORY.add(offset + 1), ATTRIBUTE);
    }
    state.col += 1;
}

fn next_line(state: &mut Vga) {
    state.col = 0;
    state.row += 1;
}

fn clear() {
    for cell in 0..WIDTH * HEIGHT {
        let offset = cell * 2;
        unsafe {
            core::ptr::write_volatile(MEMORY.add(offset), b' ');
            core::ptr::write_volatile(MEMORY.add(offset + 1), ATTRIBUTE);
        }
    }
}
