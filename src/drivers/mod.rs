pub mod block;
pub mod framework;
#[cfg(target_arch = "x86_64")]
pub mod keyboard;
pub mod serial;

pub fn init() {
    framework::init();
    framework::register(framework::Driver {
        name: "serial",
        class: framework::Class::Serial,
        state: framework::State::Ready,
    });
    framework::register(framework::Driver {
        name: "clock",
        class: framework::Class::Clock,
        state: framework::State::Ready,
    });
    framework::register(framework::Driver {
        name: "framebuffer",
        class: framework::Class::Display,
        state: framework::State::Ready,
    });
    block::init();
    framework::register(framework::Driver {
        name: "norr-ram0",
        class: framework::Class::Block,
        state: framework::State::Ready,
    });
    #[cfg(target_arch = "x86_64")]
    framework::register(framework::Driver {
        name: "ps2-keyboard",
        class: framework::Class::Input,
        state: framework::State::Ready,
    });
}
