#[cfg(target_arch = "x86_64")]
pub mod audio;
pub mod block;
pub mod display;
pub mod framework;
#[cfg(target_arch = "x86_64")]
pub mod keyboard;
#[cfg(target_arch = "x86_64")]
pub mod mouse;
#[cfg(target_arch = "x86_64")]
pub mod network;
#[cfg(target_arch = "x86_64")]
pub mod pci;
#[cfg(target_arch = "x86_64")]
pub mod ps2;
pub mod serial;
#[cfg(target_arch = "x86_64")]
pub mod usb;
#[cfg(target_arch = "x86_64")]
pub mod virtio_gpu;

#[cfg(target_arch = "x86_64")]
fn ps2_state() -> framework::DeviceState {
    crate::bootlog::start(1, "initializing ps/2 controller");
    match ps2::init() {
        ps2::InitResult::Ready => {
            let status = ps2::status();
            crate::bootlog::ok_fmt(format_args!(
                "ps/2 controller ready keyboard={} mouse={} config=0x{:02x}",
                status.keyboard_port, status.mouse_port, status.config
            ));
            framework::DeviceState::Ready
        }
        ps2::InitResult::Unsupported => {
            crate::bootlog::warn("ps/2 controller unavailable; input fallback active");
            framework::DeviceState::Unsupported
        }
        ps2::InitResult::Failed => {
            crate::bootlog::warn("ps/2 controller self-test failed; input fallback active");
            framework::DeviceState::Failed
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn ps2_state() -> framework::DeviceState {
    crate::bootlog::start(1, "initializing ps/2 controller");
    crate::bootlog::warn("ps/2 controller unsupported on aarch64");
    framework::DeviceState::Unsupported
}

#[cfg(target_arch = "x86_64")]
fn keyboard_state() -> framework::DeviceState {
    crate::bootlog::start(2, "initializing ps/2 keyboard");
    match keyboard::init() {
        keyboard::InitResult::Ready => {
            crate::bootlog::ok("ps/2 keyboard ready scan-set=2 layout=us repeat=hardware");
            framework::DeviceState::Ready
        }
        keyboard::InitResult::Unsupported => {
            crate::bootlog::warn("ps/2 keyboard unavailable; keyboard events disabled");
            framework::DeviceState::Unsupported
        }
        keyboard::InitResult::Failed => {
            crate::bootlog::warn("ps/2 keyboard setup failed; keyboard events disabled");
            framework::DeviceState::Failed
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn mouse_state() -> framework::DeviceState {
    crate::bootlog::start(3, "initializing ps/2 mouse");
    match mouse::init() {
        mouse::InitResult::Ready => {
            let status = mouse::status();
            crate::bootlog::ok_fmt(format_args!(
                "ps/2 mouse ready id={} packet={} wheel={} extra-buttons={}",
                status.id, status.packet_len, status.wheel, status.extra_buttons
            ));
            framework::DeviceState::Ready
        }
        mouse::InitResult::Unsupported => {
            crate::bootlog::warn("ps/2 mouse unavailable; pointer events disabled");
            framework::DeviceState::Unsupported
        }
        mouse::InitResult::Failed => {
            crate::bootlog::warn("ps/2 mouse setup failed; pointer events disabled");
            framework::DeviceState::Failed
        }
    }
}

#[cfg(target_arch = "aarch64")]
fn mouse_state() -> framework::DeviceState {
    crate::bootlog::start(3, "initializing ps/2 mouse");
    crate::bootlog::warn("ps/2 mouse unsupported on aarch64");
    framework::DeviceState::Unsupported
}

#[cfg(target_arch = "aarch64")]
fn keyboard_state() -> framework::DeviceState {
    crate::bootlog::start(2, "initializing ps/2 keyboard");
    crate::bootlog::warn("ps/2 keyboard unsupported on aarch64");
    framework::DeviceState::Unsupported
}

pub fn init(framebuffer: Option<crate::boot::RawFramebuffer>) -> bool {
    framework::init();
    framework::contract_self_check();
    let matrix = framework::matrix_self_check();
    if matrix.passed == matrix.scenarios {
        crate::bootlog::ok_fmt(format_args!(
            "driver failure matrix passed scenarios={} absent=true timeout=true hot-unplug=true malformed=true dma=true interrupt-storm=true",
            matrix.scenarios,
        ));
    } else {
        crate::bootlog::warn_fmt(format_args!(
            "driver failure matrix incomplete passed={}/{}; fallbacks remain active",
            matrix.passed, matrix.scenarios,
        ));
    }
    #[cfg(target_arch = "x86_64")]
    ps2::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    keyboard::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    mouse::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    pci::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    audio::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    network::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    usb::hid::contract_self_check();
    #[cfg(target_arch = "x86_64")]
    serial::ns16550::contract_self_check();
    #[cfg(target_arch = "aarch64")]
    serial::pl011::contract_self_check();
    display::contract_self_check();
    block::contract_self_check();
    let mut ok = true;
    let display_state = match display::init(framebuffer) {
        display::InitResult::Ready(_) => framework::DeviceState::Ready,
        display::InitResult::Unsupported => framework::DeviceState::Unsupported,
        display::InitResult::Failed(_) => framework::DeviceState::Failed,
    };
    if !framework::register(
        framework::Driver::early(
            1,
            "serial",
            framework::Class::Serial,
            framework::BusKind::Platform,
        ),
        framework::DeviceState::Ready,
    ) {
        ok = false;
    }
    if !framework::register(
        framework::Driver::new(
            2,
            "framebuffer",
            framework::Class::Display,
            framework::BusKind::Platform,
        ),
        display_state,
    ) {
        ok = false;
    }
    let block_state = if block::init() {
        framework::DeviceState::Ready
    } else {
        framework::DeviceState::Failed
    };
    if !framework::register(
        framework::Driver::new(
            3,
            "ramdisk0",
            framework::Class::Block,
            framework::BusKind::Platform,
        ),
        block_state,
    ) {
        ok = false;
    }
    if !framework::register(
        framework::Driver::new(
            4,
            "ps2-controller",
            framework::Class::Input,
            framework::BusKind::Ps2,
        ),
        ps2_state(),
    ) {
        ok = false;
    }
    if !framework::register(
        framework::Driver::new(
            5,
            "ps2-keyboard",
            framework::Class::Input,
            framework::BusKind::Ps2,
        ),
        keyboard_state(),
    ) {
        ok = false;
    }
    if !framework::register(
        framework::Driver::new(
            6,
            "ps2-mouse",
            framework::Class::Input,
            framework::BusKind::Ps2,
        ),
        mouse_state(),
    ) {
        ok = false;
    }
    ok
}

pub fn poll() {
    #[cfg(target_arch = "x86_64")]
    {
        keyboard::poll();
        mouse::poll();
        usb::xhci::poll();
        audio::poll();
        network::poll();
    }
}

pub fn runtime_init(framebuffer: Option<crate::boot::RawFramebuffer>) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        usb::xhci::contract_self_check();
        crate::bootlog::start(1, "probing PCI USB host controllers");
        let state = match usb::xhci::init() {
            usb::xhci::InitResult::Ready(status) => {
                crate::bootlog::ok_fmt(format_args!(
                    "xHCI ready pci={:02x}:{:02x}.{} vendor=0x{:04x} device=0x{:04x} bar=0x{:x} slots={} ports={} connected={} interrupters={} addr64={} devices={} configured={}",
                    status.bus,
                    status.slot,
                    status.function,
                    status.vendor,
                    status.device,
                    status.bar,
                    status.capabilities.slots,
                    status.capabilities.ports,
                    status.connected_ports,
                    status.capabilities.interrupters,
                    status.capabilities.address64,
                    status.devices,
                    status.configured_devices,
                ));
                if let Some(device) = status.first_device {
                    crate::bootlog::ok_fmt(format_args!(
                        "USB device slot={} address={} speed={} vid=0x{:04x} pid=0x{:04x} configuration={} class={} hub-ports={} hid={}",
                        device.slot,
                        device.address,
                        device.speed.name(),
                        device.vendor_id,
                        device.product_id,
                        device.configuration,
                        device.class.name(),
                        device.hub_ports,
                        device.hid.name(),
                    ));
                }
                framework::DeviceState::Ready
            }
            usb::xhci::InitResult::Unsupported => {
                crate::bootlog::warn("xHCI controller not found; USB stack deferred");
                framework::DeviceState::Unsupported
            }
            usb::xhci::InitResult::Failed(error) => {
                crate::bootlog::warn_fmt(format_args!(
                    "xHCI probe/reset failed: {:?}; USB stack deferred",
                    error
                ));
                framework::DeviceState::Failed
            }
        };
        let mut ok = framework::register(
            framework::Driver::new(
                7,
                "xhci",
                framework::Class::UsbHost,
                framework::BusKind::Pci,
            ),
            state,
        );
        if state == framework::DeviceState::Ready {
            crate::bootlog::start(2, "handing PCI xHCI to userspace service supervisor");
            ok &= crate::service::risky_driver_handoff();
        }

        virtio_gpu::contract_self_check();
        crate::bootlog::start(2, "probing PCI virtio-gpu controller");
        let gpu_state = match virtio_gpu::init(framebuffer) {
            virtio_gpu::InitResult::Ready(status) => {
                crate::bootlog::ok_fmt(format_args!(
                    "virtio-gpu ready pci={:02x}:{:02x}.{} vendor=0x{:04x} device=0x{:04x} irq={} controlq={} scanout={}x{} enabled={} 2d={}",
                    status.bus,
                    status.slot,
                    status.function,
                    status.vendor,
                    status.device,
                    status.irq,
                    status.control_queue,
                    status.scanout_width,
                    status.scanout_height,
                    status.scanout_enabled,
                    status.two_d,
                ));
                framework::DeviceState::Ready
            }
            virtio_gpu::InitResult::Unsupported => {
                crate::bootlog::warn(
                    "virtio-gpu controller not found; firmware framebuffer retained",
                );
                framework::DeviceState::Unsupported
            }
            virtio_gpu::InitResult::Failed(error) => {
                crate::bootlog::warn_fmt(format_args!(
                    "virtio-gpu probe failed: {:?}; firmware framebuffer retained",
                    error
                ));
                framework::DeviceState::Failed
            }
        };
        ok &= framework::register(
            framework::Driver::new(
                10,
                "virtio-gpu",
                framework::Class::Display,
                framework::BusKind::Pci,
            ),
            gpu_state,
        );

        crate::bootlog::start(2, "probing PCI AC'97 audio controller");
        let audio_state = match audio::init() {
            audio::InitResult::Ready(status) => {
                crate::bootlog::ok_fmt(format_args!(
                    "AC'97 audio ready pci={:02x}:{:02x}.{} vendor=0x{:04x} device=0x{:04x} nam=0x{:04x} nabm=0x{:04x} irq={} codec=0x{:04x}:0x{:04x} ext=0x{:04x} pcm={}Hz/{}ch/{}bit ring={}x{} running={}",
                    status.bus,
                    status.slot,
                    status.function,
                    status.vendor,
                    status.device,
                    status.nam,
                    status.nabm,
                    status.irq,
                    status.codec_vendor1,
                    status.codec_vendor2,
                    status.extended_audio_id,
                    status.format.sample_rate,
                    status.format.channels,
                    status.format.bits,
                    status.ring_entries,
                    status.buffer_bytes,
                    status.running,
                ));
                framework::DeviceState::Ready
            }
            audio::InitResult::Unsupported => {
                crate::bootlog::warn("AC'97 audio controller not found; silent fallback active");
                framework::DeviceState::Unsupported
            }
            audio::InitResult::Failed(error) => {
                crate::bootlog::warn_fmt(format_args!(
                    "AC'97 audio probe failed: {:?}; silent fallback active",
                    error
                ));
                framework::DeviceState::Failed
            }
        };
        ok &= framework::register(
            framework::Driver::new(
                8,
                "ac97-audio",
                framework::Class::Audio,
                framework::BusKind::Pci,
            ),
            audio_state,
        );
        crate::bootlog::start(3, "probing PCI virtio-net controllers");
        let network_state = match network::init() {
            network::InitResult::Ready(status) => {
                crate::bootlog::ok_fmt(format_args!(
                    "virtio-net ready pci={:02x}:{:02x}.{} vendor=0x{:04x} device=0x{:04x} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} link={} irq={} interrupts={} checksum=software rxq={} txq={}",
                    status.bus,
                    status.slot,
                    status.function,
                    status.vendor,
                    status.device,
                    status.mac[0],
                    status.mac[1],
                    status.mac[2],
                    status.mac[3],
                    status.mac[4],
                    status.mac[5],
                    status.link_up,
                    status.irq,
                    status.interrupts,
                    status.rx_queue,
                    status.tx_queue,
                ));
                if !status.interrupts {
                    crate::bootlog::warn(
                        "virtio-net IRQ unavailable; using bounded polling for completions",
                    );
                }
                framework::DeviceState::Ready
            }
            network::InitResult::Unsupported => {
                crate::bootlog::warn("virtio-net controller not found; networking deferred");
                framework::DeviceState::Unsupported
            }
            network::InitResult::Failed(error) => {
                crate::bootlog::warn_fmt(format_args!(
                    "virtio-net probe failed: {:?}; networking deferred",
                    error
                ));
                framework::DeviceState::Failed
            }
        };
        ok &= framework::register(
            framework::Driver::new(
                9,
                "virtio-net",
                framework::Class::Network,
                framework::BusKind::Pci,
            ),
            network_state,
        );
        ok
    }

    #[cfg(target_arch = "aarch64")]
    {
        let _ = framebuffer;
        crate::bootlog::start(1, "probing PCI USB host controllers");
        crate::bootlog::warn("xHCI controller unsupported on aarch64 bring-up");
        let mut ok = framework::register(
            framework::Driver::new(
                7,
                "xhci",
                framework::Class::UsbHost,
                framework::BusKind::Pci,
            ),
            framework::DeviceState::Unsupported,
        );
        crate::bootlog::start(2, "probing PCI AC'97 audio controller");
        crate::bootlog::warn("AC'97 audio unsupported on aarch64; silent fallback active");
        ok &= framework::register(
            framework::Driver::new(
                8,
                "ac97-audio",
                framework::Class::Audio,
                framework::BusKind::Pci,
            ),
            framework::DeviceState::Unsupported,
        );
        crate::bootlog::start(3, "probing PCI virtio-net controllers");
        crate::bootlog::warn("virtio-net unsupported on aarch64 bring-up; networking deferred");
        ok &= framework::register(
            framework::Driver::new(
                9,
                "virtio-net",
                framework::Class::Network,
                framework::BusKind::Pci,
            ),
            framework::DeviceState::Unsupported,
        );
        crate::bootlog::start(2, "probing PCI virtio-gpu controller");
        crate::bootlog::warn(
            "virtio-gpu unsupported on aarch64 bring-up; firmware framebuffer retained",
        );
        ok &= framework::register(
            framework::Driver::new(
                10,
                "virtio-gpu",
                framework::Class::Display,
                framework::BusKind::Pci,
            ),
            framework::DeviceState::Unsupported,
        );
        ok
    }
}
