use core::fmt;

use crate::{boot::RawFramebuffer, framebuffer};

#[derive(Clone, Copy)]
pub enum Status {
    Ok,
    Fail,
    Warn,
    Info,
    Spin(u8),
}

static mut ACTIVE_MESSAGE: Option<&'static str> = None;
static mut SPIN_FRAME: u8 = 0;
static mut QUICKINIT_FRAMEBUFFER: Option<RawFramebuffer> = None;
static mut QUICKINIT_STAGE: &'static str = "starting quickinit";
static mut QUICKINIT_PERCENT: u8 = 0;
static mut QUICKINIT_ACTIVE: bool = false;

const QUICKINIT_WIDTH: usize = 54;
const QUICKINIT_HEIGHT: usize = 9;

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Ok => "  OK  ",
            Status::Fail => "FAILED",
            Status::Warn => " WARN ",
            Status::Info => " INFO ",
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
            Status::Info => "\x1b[97m",
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

pub fn info(message: &str) {
    status(Status::Info, message);
}

#[cfg_attr(target_arch = "aarch64", allow(dead_code))]
pub fn info_fmt(args: fmt::Arguments) {
    status_fmt(Status::Info, args);
}

pub fn quickinit_overlay_begin(raw: Option<RawFramebuffer>) {
    unsafe {
        QUICKINIT_FRAMEBUFFER = raw;
        QUICKINIT_STAGE = "starting quickinit";
        QUICKINIT_PERCENT = 0;
        QUICKINIT_ACTIVE = raw.is_some();
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
            "handoff verified"
        } else {
            "boot failure"
        };
        if success {
            QUICKINIT_PERCENT = 100;
        }
    }
    redraw_quickinit_overlay();
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

pub fn clear_quickinit_overlay() {
    unsafe {
        if !QUICKINIT_ACTIVE {
            return;
        }
        let Some(raw) = QUICKINIT_FRAMEBUFFER else {
            return;
        };
        let Some((left, top)) = quickinit_overlay_geometry(raw) else {
            return;
        };
        let mut fb = framebuffer::init(raw);
        for row in 0..QUICKINIT_HEIGHT {
            for column in 0..QUICKINIT_WIDTH {
                put_overlay_char(&mut fb, left + column, top + row, b' ', 0xd8dee9);
            }
        }
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

fn draw_quickinit_overlay(raw: RawFramebuffer, stage: &str, percent: u8) {
    let Some((left, top)) = quickinit_overlay_geometry(raw) else {
        return;
    };
    let mut fb = framebuffer::init(raw);
    let border = 0x5f87ff;
    let title = 0x9cdcfe;
    let text = 0xd8dee9;
    let progress = 0x7ee787;
    let percent_color = 0xffd580;

    for row in 1..QUICKINIT_HEIGHT - 1 {
        for column in 1..QUICKINIT_WIDTH - 1 {
            put_overlay_char(&mut fb, left + column, top + row, b' ', text);
        }
    }
    for column in 0..QUICKINIT_WIDTH {
        put_overlay_char(&mut fb, left + column, top, b'-', border);
        put_overlay_char(
            &mut fb,
            left + column,
            top + QUICKINIT_HEIGHT - 1,
            b'-',
            border,
        );
    }
    for row in 1..QUICKINIT_HEIGHT - 1 {
        put_overlay_char(&mut fb, left, top + row, b'|', border);
        put_overlay_char(&mut fb, left + QUICKINIT_WIDTH - 1, top + row, b'|', border);
    }
    put_overlay_char(&mut fb, left, top, b'+', border);
    put_overlay_char(&mut fb, left + QUICKINIT_WIDTH - 1, top, b'+', border);
    put_overlay_char(&mut fb, left, top + QUICKINIT_HEIGHT - 1, b'+', border);
    put_overlay_char(
        &mut fb,
        left + QUICKINIT_WIDTH - 1,
        top + QUICKINIT_HEIGHT - 1,
        b'+',
        border,
    );

    let right = left + QUICKINIT_WIDTH - 1;
    put_overlay_text(&mut fb, left + 3, top + 1, "QUICKINIT", title, true, right);
    put_overlay_text(&mut fb, left + 3, top + 3, "stage: ", text, false, right);
    put_overlay_text(&mut fb, left + 10, top + 3, stage, text, false, right);

    let bar_width = QUICKINIT_WIDTH - 16;
    let filled = bar_width * percent as usize / 100;
    put_overlay_char(&mut fb, left + 3, top + 5, b'[', border);
    for index in 0..bar_width {
        put_overlay_char(
            &mut fb,
            left + 4 + index,
            top + 5,
            if index < filled { b'#' } else { b'.' },
            if index < filled { progress } else { 0x68707c },
        );
    }
    put_overlay_char(&mut fb, left + 4 + bar_width, top + 5, b']', border);
    put_overlay_text(
        &mut fb,
        left + 7 + bar_width,
        top + 5,
        " ",
        percent_color,
        false,
        right,
    );
    put_overlay_percent(
        &mut fb,
        left + 8 + bar_width,
        top + 5,
        percent,
        percent_color,
    );
    put_overlay_text(
        &mut fb,
        left + 3,
        top + 7,
        "kernel -> userspace handoff",
        0x8b949e,
        false,
        right,
    );
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

fn put_overlay_percent(
    fb: &mut framebuffer::Fb,
    column: usize,
    row: usize,
    percent: u8,
    color: u32,
) {
    let percent = percent.min(100);
    if percent == 100 {
        put_overlay_char(fb, column, row, b'1', color);
        put_overlay_char(fb, column + 1, row, b'0', color);
        put_overlay_char(fb, column + 2, row, b'0', color);
    } else if percent >= 10 {
        put_overlay_char(fb, column, row, b'0' + percent / 10, color);
        put_overlay_char(fb, column + 1, row, b'0' + percent % 10, color);
    } else {
        put_overlay_char(fb, column, row, b'0' + percent, color);
    }
    put_overlay_char(fb, column + 3, row, b'%', color);
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
