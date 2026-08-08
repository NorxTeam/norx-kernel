use core::fmt::{self, Write};

const WIDTH: usize = 80;
const HEIGHT: usize = 25;
const MEMORY_SIZE: usize = WIDTH * HEIGHT * 2;
const ATTRIBUTE: u8 = 0x07;

fn memory() -> Option<crate::io::MmioRegion> {
    unsafe { crate::io::MmioRegion::new(0xb8000, MEMORY_SIZE) }
}

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

pub fn disable() {
    unsafe {
        STATE = None;
    }
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
            } else if value == b'K' {
                clear_row(state);
                state.ansi = 0;
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
        scroll();
        state.row = HEIGHT - 1;
    }

    let offset = (state.row * WIDTH + state.col) * 2;
    let Some(memory) = memory() else {
        return;
    };
    let _ = memory.write_u8(offset, value);
    let _ = memory.write_u8(offset + 1, ATTRIBUTE);
    state.col += 1;
}

fn next_line(state: &mut Vga) {
    state.col = 0;
    state.row += 1;
}

fn clear() {
    let Some(memory) = memory() else {
        return;
    };
    for cell in 0..WIDTH * HEIGHT {
        let offset = cell * 2;
        let _ = memory.write_u8(offset, b' ');
        let _ = memory.write_u8(offset + 1, ATTRIBUTE);
    }
}

fn clear_row(state: &Vga) {
    if state.row >= HEIGHT {
        return;
    }
    let Some(memory) = memory() else {
        return;
    };
    for col in state.col..WIDTH {
        let offset = (state.row * WIDTH + col) * 2;
        let _ = memory.write_u8(offset, b' ');
        let _ = memory.write_u8(offset + 1, ATTRIBUTE);
    }
}

fn scroll() {
    let row_bytes = WIDTH * 2;
    let Some(memory) = memory() else {
        return;
    };
    for row in 1..HEIGHT {
        for byte in 0..row_bytes {
            let Some(value) = memory.read_u8(row * row_bytes + byte) else {
                return;
            };
            let _ = memory.write_u8((row - 1) * row_bytes + byte, value);
        }
    }
    for cell in (HEIGHT - 1) * WIDTH..HEIGHT * WIDTH {
        let offset = cell * 2;
        let _ = memory.write_u8(offset, b' ');
        let _ = memory.write_u8(offset + 1, ATTRIBUTE);
    }
}
