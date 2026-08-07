use core::fmt;

#[derive(Clone, Copy)]
pub enum Status {
    Ok,
    Fail,
    Warn,
    Info,
    Spin(u8),
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Ok => "  OK  ",
            Status::Fail => "FAILED",
            Status::Warn => " WARN ",
            Status::Info => " INFO ",
            Status::Spin(frame) => match frame & 3 {
                0 => "****  ",
                1 => " **** ",
                2 => "  ****",
                _ => " **** ",
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
 _   _  ____  ____  __  __
| \\ | |/ __ \\|  _ \\|  \\/  |
|  \\| | |  | | |_) | |\\/| |
| |\\  | |__| |  _ <| |  | |
|_| \\_|\\____/|_| \\_\\_|  |_|
\x1b[0m"
    );
}

pub fn status(status: Status, message: &str) {
    line_start();
    prefix(status);
    crate::kprintln!("{}", message);
}

pub fn status_fmt(status: Status, args: fmt::Arguments) {
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

pub fn start(frame: u8, message: &str) {
    line_start();
    prefix(Status::Spin(frame));
    crate::kprint!("{}", message);
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
