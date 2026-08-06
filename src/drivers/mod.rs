pub mod block;
pub mod framework;
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
        name: "norx-ram0",
        class: framework::Class::Block,
        state: framework::State::Ready,
    });
}
