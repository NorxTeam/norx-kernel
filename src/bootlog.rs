use core::fmt;

use crate::{boot::RawFramebuffer, framebuffer};

#[derive(Clone, Copy)]
pub enum Status {
    Ok,
    Fail,
    Warn,
    Spin(u8),
}

static mut ACTIVE_MESSAGE: Option<&'static str> = None;
static mut SPIN_FRAME: u8 = 0;
static mut QUICKINIT_FRAMEBUFFER: Option<RawFramebuffer> = None;
static mut QUICKINIT_STAGE: &'static str = "starting quickinit";
static mut QUICKINIT_PERCENT: u8 = 0;
static mut QUICKINIT_ACTIVE: bool = false;
static mut QUICKINIT_DRAWN: bool = false;
static mut QUICKINIT_RENDERED_STAGE: &'static str = "";
static mut QUICKINIT_RENDERED_PERCENT: u8 = 0;
static mut QUICKINIT_RENDERED_PROGRESS: bool = false;

const QUICKINIT_WIDTH: usize = 54;
const QUICKINIT_HEIGHT: usize = 9;

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Ok => "  OK  ",
            Status::Fail => "FAILED",
            Status::Warn => " WARN ",
            Status::Spin(frame) => match frame & 7 {
                0 => "  >   ",
                1 => "   >  ",
                2 => "    > ",
                3 => "     >",
                4 => "    < ",
                5 => "   <  ",
                6 => "  <   ",
                _ => " <    ",
            },
        }
    }

    fn color(self) -> &'static str {
        match self {
            Status::Ok => "\x1b[92m",
            Status::Fail => "\x1b[91m",
            Status::Warn => "\x1b[93m",
            Status::Spin(_) => "\x1b[97m",
        }
    }
}

pub fn title() {
    crate::kprintln!(
        "\x1b[97m\
▄▄▄    ▄▄▄   ▄▄▄▄▄   ▄▄▄▄▄▄▄   ▄▄▄   ▄▄▄   \n\
████▄  ███ ▄███████▄ ███▀▀███▄ ████▄████   \n\
███▀██▄███ ███   ███ ███▄▄███▀  ▀█████▀    \n\
███  ▀████ ███▄▄▄███ ███▀▀██▄  ▄███████▄   \n\
███    ███  ▀█████▀  ███  ▀███ ███▀ ▀███   \n\
                                                              \n\
\x1b[0m"
    );
}

pub fn status(status: Status, message: &str) {
    clear_active();
    line_start();
    prefix(status);
    crate::kprintln!("{}", message);
}

pub fn status_fmt(status: Status, args: fmt::Arguments) {
    clear_active();
    line_start();
    prefix(status);
    crate::log::write(args);
    crate::kprintln!();
}

pub fn ok(message: &str) {
    status(Status::Ok, message);
}

pub fn ok_fmt(args: fmt::Arguments) {
    status_fmt(Status::Ok, args);
}

pub fn fail(message: &str) {
    status(Status::Fail, message);
}

pub fn fail_fmt(args: fmt::Arguments) {
    status_fmt(Status::Fail, args);
}

pub fn warn(message: &str) {
    status(Status::Warn, message);
}

pub fn warn_fmt(args: fmt::Arguments) {
    status_fmt(Status::Warn, args);
}

pub fn quickinit_overlay_begin(raw: Option<RawFramebuffer>) {
    unsafe {
        QUICKINIT_FRAMEBUFFER = raw;
        QUICKINIT_STAGE = "starting quickinit";
        QUICKINIT_PERCENT = 0;
        QUICKINIT_ACTIVE = raw.is_some();
        QUICKINIT_DRAWN = false;
        QUICKINIT_RENDERED_STAGE = "";
        QUICKINIT_RENDERED_PERCENT = 0;
        QUICKINIT_RENDERED_PROGRESS = false;
    }
    redraw_quickinit_overlay();
}

pub fn quickinit_overlay_stage(stage: &'static str, percent: u8) {
    unsafe {
        QUICKINIT_STAGE = stage;
        QUICKINIT_PERCENT = percent.min(100);
    }
    redraw_quickinit_overlay();
}

pub fn quickinit_overlay_finish(success: bool) {
    unsafe {
        QUICKINIT_STAGE = if success {
            "starting system services"
        } else {
            "recovery path"
        };
        QUICKINIT_PERCENT = if success { 92 } else { 88 };
    }
    redraw_quickinit_overlay();
}

pub fn quickinit_overlay_complete(success: bool) {
    unsafe {
        QUICKINIT_STAGE = if success {
            "system ready"
        } else {
            "recovery ready"
        };
        QUICKINIT_PERCENT = 100;
    }
    redraw_quickinit_overlay();
}

pub fn quickinit_overlay_crash(stage: &'static str) {
    unsafe {
        if !QUICKINIT_ACTIVE {
            return;
        }
        QUICKINIT_STAGE = stage;
        QUICKINIT_PERCENT = 100;
    }
    redraw_quickinit_overlay();
}

#[allow(dead_code)]
pub fn quickinit_overlay_shell() {
    let redraw;
    unsafe {
        redraw = core::ptr::read(core::ptr::addr_of!(QUICKINIT_FRAMEBUFFER)).is_some();
        QUICKINIT_ACTIVE = false;
        QUICKINIT_DRAWN = false;
    }
    if redraw {
        crate::log::redraw_console();
    }
}

pub fn redraw_quickinit_overlay() {
    unsafe {
        if !QUICKINIT_ACTIVE {
            return;
        }
        let Some(raw) = QUICKINIT_FRAMEBUFFER else {
            return;
        };
        draw_quickinit_overlay(raw, QUICKINIT_STAGE, QUICKINIT_PERCENT);
    }
}

pub fn quickinit_overlay_contains(column: usize, row: usize) -> bool {
    unsafe {
        QUICKINIT_ACTIVE
            && QUICKINIT_FRAMEBUFFER
                .and_then(quickinit_overlay_geometry)
                .is_some_and(|(left, top)| {
                    column >= left
                        && column < left + QUICKINIT_WIDTH
                        && row >= top
                        && row < top + QUICKINIT_HEIGHT
                })
    }
}

pub fn start(frame: u8, message: &'static str) {
    unsafe {
        ACTIVE_MESSAGE = Some(message);
        SPIN_FRAME = frame;
    }
    render_active();
}

pub fn pulse() {
    unsafe {
        let active = ACTIVE_MESSAGE;
        if active.is_none() {
            return;
        }
        SPIN_FRAME = SPIN_FRAME.wrapping_add(1);
    }
    render_active();
}

fn render_active() {
    unsafe {
        let active = ACTIVE_MESSAGE;
        if let Some(message) = active {
            line_start();
            prefix(Status::Spin(SPIN_FRAME));
            crate::kprint!("{}", message);
        }
    }
}

fn clear_active() {
    unsafe {
        ACTIVE_MESSAGE = None;
    }
}

fn line_start() {
    crate::kprint!("\r\x1b[2K");
}

fn prefix(status: Status) {
    crate::kprint!(
        "\x1b[90m[\x1b[0m{}{}\x1b[0m\x1b[90m]\x1b[0m ",
        status.color(),
        status.label()
    );
}

fn draw_quickinit_overlay(raw: RawFramebuffer, stage: &'static str, percent: u8) {
    let Some((left, top)) = quickinit_overlay_geometry(raw) else {
        return;
    };
    let mut fb = framebuffer::init(raw);
    let border = 0x5f87ff;
    let title = 0x9cdcfe;
    let text = 0xd8dee9;
    let progress = 0x7ee787;
    let percent_color = 0xffd580;

    let first_draw = unsafe { !QUICKINIT_DRAWN };
    if first_draw {
        for row in 1..QUICKINIT_HEIGHT - 1 {
            for column in 1..QUICKINIT_WIDTH - 1 {
                put_overlay_char(&mut fb, left + column, top + row, b' ', text);
            }
        }
        for column in 0..QUICKINIT_WIDTH {
            put_overlay_symbol(&mut fb, left + column, top, '─', border);
            put_overlay_symbol(
                &mut fb,
                left + column,
                top + QUICKINIT_HEIGHT - 1,
                '─',
                border,
            );
        }
        for row in 1..QUICKINIT_HEIGHT - 1 {
            put_overlay_symbol(&mut fb, left, top + row, '│', border);
            put_overlay_symbol(&mut fb, left + QUICKINIT_WIDTH - 1, top + row, '│', border);
        }
        put_overlay_symbol(&mut fb, left, top, '┌', border);
        put_overlay_symbol(&mut fb, left + QUICKINIT_WIDTH - 1, top, '┐', border);
        put_overlay_symbol(&mut fb, left, top + QUICKINIT_HEIGHT - 1, '└', border);
        put_overlay_symbol(
            &mut fb,
            left + QUICKINIT_WIDTH - 1,
            top + QUICKINIT_HEIGHT - 1,
            '┘',
            border,
        );
        put_overlay_text(
            &mut fb,
            left + 3,
            top + 1,
            "QUICKINIT",
            title,
            true,
            left + QUICKINIT_WIDTH - 1,
        );
        unsafe { QUICKINIT_DRAWN = true };
    }

    let right = left + QUICKINIT_WIDTH - 1;
    let stage_changed = unsafe { QUICKINIT_RENDERED_STAGE != stage };
    if first_draw || stage_changed {
        put_overlay_text_padded(&mut fb, left + 3, top + 3, stage, text, right);
        unsafe { QUICKINIT_RENDERED_STAGE = stage };
    }

    let bar_width = QUICKINIT_WIDTH - 16;
    let filled = bar_width * percent as usize / 100;
    let old_percent = unsafe { QUICKINIT_RENDERED_PERCENT };
    let old_filled = bar_width * old_percent as usize / 100;
    if first_draw || unsafe { !QUICKINIT_RENDERED_PROGRESS } {
        put_overlay_symbol(&mut fb, left + 3, top + 5, '│', border);
        put_overlay_symbol(&mut fb, left + 4 + bar_width, top + 5, '│', border);
    }
    for index in 0..bar_width {
        if first_draw || (index < filled) != (index < old_filled) {
            put_overlay_symbol(
                &mut fb,
                left + 4 + index,
                top + 5,
                if index < filled { '■' } else { '□' },
                if index < filled { progress } else { 0x68707c },
            );
        }
    }
    let new_percent = percent_cells(percent);
    let old_percent = percent_cells(old_percent);
    for (offset, (&new, &old)) in new_percent.iter().zip(old_percent.iter()).enumerate() {
        if first_draw || new != old {
            put_overlay_char(
                &mut fb,
                left + 8 + bar_width + offset,
                top + 5,
                new,
                percent_color,
            );
        }
    }
    unsafe {
        QUICKINIT_RENDERED_PERCENT = percent;
        QUICKINIT_RENDERED_PROGRESS = true;
    }
}

fn quickinit_overlay_geometry(raw: RawFramebuffer) -> Option<(usize, usize)> {
    let columns = raw.width as usize / framebuffer::TERM_W;
    let rows = raw.height as usize / framebuffer::TERM_H;
    (columns >= QUICKINIT_WIDTH + 2 && rows >= QUICKINIT_HEIGHT + 2).then_some((
        (columns - QUICKINIT_WIDTH) / 2,
        (rows - QUICKINIT_HEIGHT) / 2,
    ))
}

fn put_overlay_text(
    fb: &mut framebuffer::Fb,
    column: usize,
    row: usize,
    text: &str,
    color: u32,
    bold: bool,
    right: usize,
) {
    for (offset, byte) in text.bytes().enumerate() {
        if column + offset >= right {
            break;
        }
        fb.term_char(
            (column + offset) * framebuffer::TERM_W,
            row * framebuffer::TERM_H,
            byte,
            color,
            0x000000,
            bold,
        );
    }
}

fn put_overlay_text_padded(
    fb: &mut framebuffer::Fb,
    column: usize,
    row: usize,
    text: &str,
    color: u32,
    right: usize,
) {
    let width = right.saturating_sub(column);
    for offset in 0..width {
        let byte = text.as_bytes().get(offset).copied().unwrap_or(b' ');
        put_overlay_char(fb, column + offset, row, byte, color);
    }
}

fn percent_cells(percent: u8) -> [u8; 4] {
    let percent = percent.min(100);
    if percent == 100 {
        [b'1', b'0', b'0', b'%']
    } else if percent >= 10 {
        [b' ', b'0' + percent / 10, b'0' + percent % 10, b'%']
    } else {
        [b' ', b' ', b'0' + percent, b'%']
    }
}

fn put_overlay_char(fb: &mut framebuffer::Fb, column: usize, row: usize, byte: u8, color: u32) {
    fb.term_char(
        column * framebuffer::TERM_W,
        row * framebuffer::TERM_H,
        byte,
        color,
        0x000000,
        false,
    );
}

fn put_overlay_symbol(
    fb: &mut framebuffer::Fb,
    column: usize,
    row: usize,
    symbol: char,
    color: u32,
) {
    fb.term_codepoint(
        column * framebuffer::TERM_W,
        row * framebuffer::TERM_H,
        symbol as u32,
        color,
        0x000000,
        false,
    );
}
