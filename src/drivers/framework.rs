const MAX_DRIVERS: usize = 32;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Class {
    Clock,
    Display,
    Input,
    Serial,
    Block,
    Filesystem,
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum State {
    Ready,
    MissingHardware,
    Error,
}

#[derive(Clone, Copy)]
pub struct Driver {
    pub name: &'static str,
    pub class: Class,
    pub state: State,
}

static mut DRIVERS: [Option<Driver>; MAX_DRIVERS] = [None; MAX_DRIVERS];
static mut LEN: usize = 0;

pub fn init() {
    unsafe {
        LEN = 0;
        DRIVERS = [None; MAX_DRIVERS];
    }
}

pub fn register(driver: Driver) -> bool {
    unsafe {
        if LEN == MAX_DRIVERS {
            return false;
        }
        DRIVERS[LEN] = Some(driver);
        LEN += 1;
        true
    }
}

pub fn list(mut f: impl FnMut(Driver)) {
    unsafe {
        let drivers = &raw const DRIVERS;
        for i in 0..LEN {
            if let Some(driver) = (*drivers)[i] {
                f(driver);
            }
        }
    }
}

pub fn class_name(class: Class) -> &'static str {
    match class {
        Class::Clock => "clock",
        Class::Display => "display",
        Class::Input => "input",
        Class::Serial => "serial",
        Class::Block => "block",
        Class::Filesystem => "filesystem",
    }
}

pub fn state_name(state: State) -> &'static str {
    match state {
        State::Ready => "ready",
        State::MissingHardware => "missing",
        State::Error => "error",
    }
}
