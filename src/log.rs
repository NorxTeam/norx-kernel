use core::fmt::{self, Write};

use crate::{boot::RawFramebuffer, framebuffer};

struct Console {
    raw: RawFramebuffer,
    col: usize,
    row: usize,
    cols: usize,
    rows: usize,
    ansi: u8,
    sgr: [u8; 4],
    sgr_len: usize,
    fg: u32,
    bg: u32,
}

static mut CONSOLE: Option<Console> = None;

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! kprintln {
    () => {
        $crate::kprint!("\n")
    };
    ($fmt:expr) => {
        $crate::kprint!(concat!($fmt, "\n"))
    };
    ($fmt:expr, $($arg:tt)*) => {
        $crate::kprint!(concat!($fmt, "\n"), $($arg)*)
    };
}

pub fn init() {
    #[cfg(target_arch = "x86_64")]
    crate::vga::init();
}

pub fn init_framebuffer(raw: RawFramebuffer) {
    unsafe {
        framebuffer::init(raw).clear(0x000000);
        CONSOLE = Some(Console {
            raw,
            col: 2,
            row: 2,
            cols: raw.width as usize / framebuffer::TERM_W,
            rows: raw.height as usize / framebuffer::TERM_H,
            ansi: 0,
            sgr: [0; 4],
            sgr_len: 0,
            fg: 0xd8dee9,
            bg: 0x000000,
        });
    }
}

pub fn write(args: fmt::Arguments) {
    crate::drivers::serial::write(args);
    #[cfg(target_arch = "x86_64")]
    crate::vga::write(args);
    let _ = Screen.write_fmt(args);
}

struct Screen;

impl Write for Screen {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            screen_byte(byte);
        }
        Ok(())
    }
}

fn screen_byte(byte: u8) {
    unsafe {
        let Some(slot) = (&raw mut CONSOLE).as_mut() else {
            return;
        };
        let Some(console) = slot.as_mut() else {
            return;
        };

        draw_cursor(console, false);

        if console.ansi != 0 {
            handle_ansi(console, byte);
            draw_cursor(console, true);
            return;
        }

        match byte {
            0x1b => console.ansi = 1,
            b'\n' => {
                console.col = 2;
                console.row += 1;
            }
            b'\r' => console.col = 2,
            8 => {
                if console.col > 2 {
                    console.col -= 1;
                    let mut fb = framebuffer::init(console.raw);
                    fb.term_char(
                        console.col * framebuffer::TERM_W,
                        console.row * framebuffer::TERM_H,
                        b' ',
                        console.fg,
                        console.bg,
                    );
                }
            }
            byte => {
                let mut fb = framebuffer::init(console.raw);
                fb.term_char(
                    console.col * framebuffer::TERM_W,
                    console.row * framebuffer::TERM_H,
                    byte,
                    console.fg,
                    console.bg,
                );
                console.col += 1;
                if console.col >= console.cols {
                    console.col = 2;
                    console.row += 1;
                }
            }
        }

        if console.row >= console.rows {
            let mut fb = framebuffer::init(console.raw);
            fb.clear(0x000000);
            console.col = 2;
            console.row = 2;
        }

        draw_cursor(console, true);
    }
}

fn handle_ansi(console: &mut Console, byte: u8) {
    match (console.ansi, byte) {
        (1, b'[') => console.ansi = 2,
        (2, b'0'..=b'9') if console.sgr_len < console.sgr.len() => {
            console.sgr[console.sgr_len] = byte - b'0';
            console.sgr_len += 1;
        }
        (2, b';') if console.sgr_len < console.sgr.len() => {
            console.sgr[console.sgr_len] = 255;
            console.sgr_len += 1;
        }
        (2, b'm') => {
            apply_sgr(console);
            console.sgr_len = 0;
            console.ansi = 0;
        }
        (2, b'K') => {
            erase_line(console);
            console.ansi = 0;
        }
        (2, b'J') => {
            reset_screen(console);
            console.ansi = 0;
        }
        (2, b'D') => {
            console.col = console.col.saturating_sub(1).max(2);
            console.ansi = 0;
        }
        (2, b'C') => {
            if console.col + 1 < console.cols {
                console.col += 1;
            }
            console.ansi = 0;
        }
        (2, b'H') | (2, b'f') => {
            console.col = 2;
            console.row = 2;
            console.ansi = 0;
        }
        (2, b'F') => {
            console.col = console.cols.saturating_sub(1);
            console.ansi = 0;
        }
        _ => console.ansi = 0,
    }
}

fn reset_screen(console: &mut Console) {
    let mut fb = framebuffer::init(console.raw);
    fb.clear(0x000000);
    console.col = 2;
    console.row = 2;
    console.ansi = 0;
    console.sgr_len = 0;
    console.fg = 0xd8dee9;
    console.bg = 0x000000;
}

fn apply_sgr(console: &mut Console) {
    if console.sgr_len == 0 {
        console.fg = 0xd8dee9;
        console.bg = 0x000000;
        return;
    }

    let mut value = 0u8;
    let mut have = false;
    for i in 0..console.sgr_len {
        let b = console.sgr[i];
        if b == 255 {
            apply_sgr_value(console, if have { value } else { 0 });
            value = 0;
            have = false;
        } else {
            value = value.saturating_mul(10).saturating_add(b);
            have = true;
        }
    }
    apply_sgr_value(console, if have { value } else { 0 });
}

fn apply_sgr_value(console: &mut Console, value: u8) {
    match value {
        0 => {
            console.fg = 0xd8dee9;
            console.bg = 0x000000;
        }
        30..=37 => console.fg = ansi_color(value - 30, false),
        40..=47 => console.bg = ansi_color(value - 40, false),
        90..=97 => console.fg = ansi_color(value - 90, true),
        100..=107 => console.bg = ansi_color(value - 100, true),
        _ => {}
    }
}

fn ansi_color(index: u8, bright: bool) -> u32 {
    let dark = [
        0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa,
    ];
    let light = [
        0x555555, 0xff5555, 0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff,
    ];
    if bright {
        light[index as usize]
    } else {
        dark[index as usize]
    }
}

fn erase_line(console: &mut Console) {
    let mut fb = framebuffer::init(console.raw);
    for col in console.col..console.cols {
        fb.term_char(
            col * framebuffer::TERM_W,
            console.row * framebuffer::TERM_H,
            b' ',
            console.fg,
            console.bg,
        );
    }
}

fn draw_cursor(console: &Console, on: bool) {
    let mut fb = framebuffer::init(console.raw);
    fb.term_cursor(
        console.col * framebuffer::TERM_W,
        console.row * framebuffer::TERM_H,
        on,
        console.fg,
        console.bg,
    );
}
