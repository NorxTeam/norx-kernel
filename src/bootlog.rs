use core::fmt;

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
 _   _  ___  ____  __  __
| \\ | |/ _ \\|  _ \\ \\ \\/ /
|  \\| | | | | |_) | \\  /
| |\\  | | |_| |  _ <  /  \\
|_| \\_|\\___/|_| \\_\\ /_/\\_\
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

pub fn warn(message: &str) {
    status(Status::Warn, message);
}

pub fn warn_fmt(args: fmt::Arguments) {
    status_fmt(Status::Warn, args);
}

pub fn info(message: &str) {
    status(Status::Info, message);
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
        "\x1b[1;97m[\x1b[0m{}{}\x1b[0m\x1b[90m]\x1b[0m ",
        status.color(),
        status.label()
    );
}
