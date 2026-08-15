use core::fmt::{self, Write};

const LINE_MAX: usize = 128;
const ARG_MAX: usize = 8;
pub const PROTOCOL_VERSION: u8 = 1;

pub fn run() -> ! {
    let mut debugger = SerialDebugger::new();
    debugger.banner();

    loop {
        crate::drivers::poll();
        #[cfg(target_arch = "x86_64")]
        crate::net::poll();
        let _ = crate::irq::run_deferred();
        if !crate::timer::hardware_ticks() && crate::time::poll_scheduler_tick() {
            crate::sched::on_timer_tick();
        }
        if let Some(byte) = crate::drivers::serial::read() {
            debugger.byte(byte);
        }
    }
}

struct SerialDebugger {
    line: [u8; LINE_MAX],
    len: usize,
}

impl SerialDebugger {
    const fn new() -> Self {
        Self {
            line: [0; LINE_MAX],
            len: 0,
        }
    }

    fn banner(&mut self) {
        self.println("Norx serial-debugger");
        self.write_fmt(format_args!(
            "NORX_SERIAL_DEBUGGER_READY v={}\n",
            PROTOCOL_VERSION
        ));
        self.println("type help for commands");
        self.prompt();
    }

    fn byte(&mut self, byte: u8) {
        match byte {
            b'\r' => {}
            b'\n' => {
                self.write("\r\n");
                self.execute();
                self.len = 0;
                self.prompt();
            }
            8 | 127 => {
                if self.len != 0 {
                    self.len -= 1;
                    self.write("\x08 \x08");
                }
            }
            b'\t' | 0x20..=0x7e if self.len < LINE_MAX - 1 => {
                self.line[self.len] = byte;
                self.len += 1;
                let _ = crate::drivers::serial::Serial.write_char(byte as char);
            }
            _ => {}
        }
    }

    fn execute(&mut self) {
        let len = self.len;
        let line_bytes = self.line;
        let line = core::str::from_utf8(&line_bytes[..len]).unwrap_or("");
        let mut args = [""; ARG_MAX];
        let argc = split(line, &mut args);
        if argc == 0 {
            return;
        }

        match args[0] {
            "help" => self.println(
                "commands: help time sched irq hw mem paging drivers trace display block ps2 keyboard mouse input usb audio net(ping|udp|dns|tcp|tx|rx) uname vfs(list|mount|umount|lookup|namespace) ls cat write crash halt",
            ),
            "time" => show_time(self),
            "sched" => sched(self),
            "irq" => irq(self),
            "hw" => hw(self),
            "display" => display(self, &args[1..argc]),
            "block" => block(self, &args[1..argc]),
            "mem" => mem(self),
            "paging" => paging(self),
            "drivers" => drivers(self),
            "trace" => trace(self),
            #[cfg(target_arch = "x86_64")]
            "ps2" => ps2(self, &args[1..argc]),
            #[cfg(target_arch = "aarch64")]
            "ps2" => self.println("ps/2 unavailable on aarch64"),
            #[cfg(target_arch = "x86_64")]
            "keyboard" => keyboard(self, &args[1..argc]),
            #[cfg(target_arch = "aarch64")]
            "keyboard" => self.println("keyboard unavailable on aarch64"),
            #[cfg(target_arch = "x86_64")]
            "mouse" => mouse(self),
            #[cfg(target_arch = "aarch64")]
            "mouse" => self.println("mouse unavailable on aarch64"),
            #[cfg(target_arch = "x86_64")]
            "input" => input(self),
            #[cfg(target_arch = "aarch64")]
            "input" => self.println("input unavailable on aarch64"),
            #[cfg(target_arch = "x86_64")]
            "usb" => usb(self),
            #[cfg(target_arch = "aarch64")]
            "usb" => self.println("usb host controller unavailable on aarch64"),
            #[cfg(target_arch = "x86_64")]
            "audio" => audio(self, &args[1..argc]),
            #[cfg(target_arch = "aarch64")]
            "audio" => self.println("audio unavailable on aarch64; silent fallback active"),
            #[cfg(target_arch = "x86_64")]
            "net" => net(self, &args[1..argc]),
            #[cfg(target_arch = "aarch64")]
            "net" => self.println("network unavailable on aarch64; networking deferred"),
            "uname" => self.write_fmt(format_args!("Norx {} grub\n", crate::arch::NAME)),
            "vfs" => vfs(self, &args[1..argc]),
            "ls" => ls(self),
            "cat" => cat(self, &args[1..argc]),
            "write" => write_file(self, &args[1..argc]),
            "crash" => crate::crash::fatal(crate::error::KernelError {
                kind: crate::error::ErrorKind::Panic,
                code: 0xdead,
                message: "manual crash",
                detail: "requested by serial-debugger command",
                arg0: 0,
                arg1: 0,
            }),
            "halt" => crate::arch::halt(),
            command => self.write_fmt(format_args!("unknown command: {command}\n")),
        }
    }

    fn prompt(&mut self) {
        self.write("norx-debug> ");
    }

    fn println(&mut self, message: &str) {
        self.write(message);
        self.write("\n");
    }

    fn write(&mut self, message: &str) {
        crate::drivers::serial::write_str(message);
    }

    fn write_fmt(&mut self, args: fmt::Arguments) {
        let _ = crate::drivers::serial::Serial.write_fmt(args);
    }
}

fn split<'a>(line: &'a str, out: &mut [&'a str; ARG_MAX]) -> usize {
    let mut len = 0;
    for arg in line.split_ascii_whitespace() {
        if len == ARG_MAX {
            break;
        }
        out[len] = arg;
        len += 1;
    }
    len
}

fn show_time(debugger: &mut SerialDebugger) {
    if let Some(ticks) = crate::time::boot_time() {
        debugger.write_fmt(format_args!(
            "boot ticks={} now={}\n",
            ticks,
            crate::time::ticks()
        ));
    } else {
        debugger.println("time unavailable");
    }
}

fn sched(debugger: &mut SerialDebugger) {
    let status = crate::sched::status();
    debugger.write_fmt(format_args!(
        "mode={} hz={} interval={} ticks={} clock={} tasks={} current={:?} next={:?}\n",
        if crate::timer::hardware_ticks() {
            "irq"
        } else {
            "poll"
        },
        crate::time::scheduler_hz(),
        crate::time::scheduler_ticks(),
        status.timer_ticks,
        status.clock,
        status.tasks,
        status.current,
        status.next,
    ));
}

fn irq(debugger: &mut SerialDebugger) {
    let stats = crate::irq::stats();
    debugger.write_fmt(format_args!(
        "timer={} pending={} spurious={} unhandled={} deferred={} exceptions={} hard_context_violations={}\n",
        stats.timer,
        stats.timer_pending,
        stats.spurious,
        stats.unhandled,
        stats.deferred,
        stats.exceptions,
        stats.hard_context_violations,
    ));
}

fn hw(debugger: &mut SerialDebugger) {
    #[cfg(target_arch = "x86_64")]
    {
        let apic = crate::arch::apic::status();
        debugger.write_fmt(format_args!(
            "arch=x86_64 apic={} x2apic={} enabled={} software={} id={} version={} base=0x{:x} irq-timer={}\n",
            apic.present,
            apic.x2apic,
            apic.enabled,
            apic.software_enabled,
            apic.id,
            apic.version,
            apic.base,
            crate::timer::hardware_ticks(),
        ));
    }

    #[cfg(target_arch = "aarch64")]
    debugger.write_fmt(format_args!(
        "arch=aarch64 gic=false irq-timer={} lazy-pages={}\n",
        crate::timer::hardware_ticks(),
        crate::arch::supports_lazy_pages(),
    ));
}

fn display(debugger: &mut SerialDebugger, args: &[&str]) {
    match args.first().copied() {
        None => {}
        Some("modes") => {
            let mut modes = [crate::drivers::display::Mode {
                id: 0,
                width: 0,
                height: 0,
                stride: 0,
                bytes_per_pixel: 0,
                format: crate::boot::PixelFormat::Rgb,
            }; 8];
            match crate::drivers::display::modes(&mut modes) {
                Ok(count) => {
                    for mode in modes.iter().take(count) {
                        debugger.write_fmt(format_args!(
                            "mode={} {}x{} stride={} bpp={} format={:?}\n",
                            mode.id,
                            mode.width,
                            mode.height,
                            mode.stride,
                            mode.bytes_per_pixel,
                            mode.format,
                        ));
                    }
                }
                Err(error) => debugger.write_fmt(format_args!("display modes: {:?}\n", error)),
            }
        }
        Some("mode") => {
            let Some(id) = args.get(1).and_then(|value| value.parse::<u16>().ok()) else {
                debugger.println("display mode: use mode <id>");
                return;
            };
            match crate::drivers::display::set_mode(id) {
                Ok(mode) => debugger.write_fmt(format_args!(
                    "display mode selected id={} {}x{}\n",
                    mode.id, mode.width, mode.height
                )),
                Err(error) => debugger.write_fmt(format_args!("display mode: {:?}\n", error)),
            }
        }
        Some("damage") => {
            let Some(x) = args.get(1).and_then(|value| value.parse::<u32>().ok()) else {
                debugger.println("display damage: use damage <x> <y> <width> <height>");
                return;
            };
            let Some(y) = args.get(2).and_then(|value| value.parse::<u32>().ok()) else {
                debugger.println("display damage: use damage <x> <y> <width> <height>");
                return;
            };
            let Some(width) = args.get(3).and_then(|value| value.parse::<u32>().ok()) else {
                debugger.println("display damage: use damage <x> <y> <width> <height>");
                return;
            };
            let Some(height) = args.get(4).and_then(|value| value.parse::<u32>().ok()) else {
                debugger.println("display damage: use damage <x> <y> <width> <height>");
                return;
            };
            match crate::drivers::display::queue_damage(crate::drivers::display::DamageRegion {
                x,
                y,
                width,
                height,
            }) {
                Ok(()) => debugger.println("display damage queued"),
                Err(error) => debugger.write_fmt(format_args!("display damage: {:?}\n", error)),
            }
        }
        Some("flush") => match crate::drivers::display::flush() {
            Ok(count) => debugger.write_fmt(format_args!("display flushed regions={}\n", count)),
            Err(error) => debugger.write_fmt(format_args!("display flush: {:?}\n", error)),
        },
        Some("cursor") => {
            let Some(mode) = args.get(1).copied() else {
                debugger.println("display cursor: use cursor on <x> <y> <width> <height> or off");
                return;
            };
            let cursor = if mode == "off" {
                crate::drivers::display::Cursor {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                    visible: false,
                }
            } else {
                let Some(x) = args.get(2).and_then(|value| value.parse::<u32>().ok()) else {
                    debugger.println(
                        "display cursor: use cursor on <x> <y> <width> <height> or off",
                    );
                    return;
                };
                let Some(y) = args.get(3).and_then(|value| value.parse::<u32>().ok()) else {
                    debugger.println(
                        "display cursor: use cursor on <x> <y> <width> <height> or off",
                    );
                    return;
                };
                let Some(width) = args.get(4).and_then(|value| value.parse::<u32>().ok()) else {
                    debugger.println(
                        "display cursor: use cursor on <x> <y> <width> <height> or off",
                    );
                    return;
                };
                let Some(height) = args.get(5).and_then(|value| value.parse::<u32>().ok()) else {
                    debugger.println(
                        "display cursor: use cursor on <x> <y> <width> <height> or off",
                    );
                    return;
                };
                crate::drivers::display::Cursor {
                    x,
                    y,
                    width,
                    height,
                    visible: true,
                }
            };
            match crate::drivers::display::set_cursor(cursor) {
                Ok(()) => debugger.write_fmt(format_args!("display cursor visible={}\n", cursor.visible)),
                Err(error) => debugger.write_fmt(format_args!("display cursor: {:?}\n", error)),
            }
        }
        Some("edid") => {
            let mut edid = [0u8; 256];
            match crate::drivers::display::edid(&mut edid) {
                Ok(length) => debugger.write_fmt(format_args!("display edid bytes={}\n", length)),
                Err(error) => debugger.write_fmt(format_args!("display edid: {:?}\n", error)),
            }
        }
        Some(command) => debugger.write_fmt(format_args!(
            "display: unknown command {command}; use modes, mode, damage, flush, cursor, edid, or no argument\n"
        )),
    }

    if let Some(event) = crate::drivers::display::poll_hotplug() {
        debugger.write_fmt(format_args!("display hotplug={event:?}\n"));
    }
    let Some(status) = crate::drivers::display::status() else {
        debugger.println("display unavailable; serial remains active");
        return;
    };
    debugger.write_fmt(format_args!(
        "display present={} mode={} modes={} edid={} damage={} cursor={} hotplug={} flushes={}\n",
        status.present,
        status.active_mode,
        status.mode_count,
        status.edid_available,
        status.pending_damage,
        status.cursor_visible,
        status.hotplug_generation,
        status.flushes,
    ));
}

fn mem(debugger: &mut SerialDebugger) {
    let stats = crate::memory::stats();
    debugger.write_fmt(format_args!(
        "ranges={} usable={}KiB allocated_frames={} next=0x{:x}\n",
        stats.ranges,
        stats.usable_pages * 4,
        stats.allocated_frames,
        stats.next_frame,
    ));
}

fn paging(debugger: &mut SerialDebugger) {
    let stats = crate::paging::stats();
    debugger.write_fmt(format_args!(
        "direct_map={} base=0x{:x} bytes={}KiB norx_cr3={} cr3=0x{:x} tables={}/{} lazy_pages={}\n",
        stats.direct_map_ready,
        stats.direct_map_base,
        stats.direct_map_bytes / 1024,
        stats.norx_cr3_ready,
        stats.norx_cr3,
        stats.table_pages_used,
        stats.table_pages_total,
        stats.lazy_pages,
    ));
    #[cfg(target_arch = "aarch64")]
    {
        let reg = crate::arch::paging::status();
        debugger.write_fmt(format_args!(
            "aarch64 ttbr0=0x{:x} ttbr1=0x{:x} tcr=0x{:x} mair=0x{:x} sctlr=0x{:x} tables={}\n",
            reg.ttbr0, reg.ttbr1, reg.tcr, reg.mair, reg.sctlr, reg.tables_used,
        ));
    }
}

fn drivers(debugger: &mut SerialDebugger) {
    crate::drivers::framework::list(|driver| {
        debugger.write_fmt(format_args!(
            "{}\t{}\t{}\t{}\t{}\n",
            crate::drivers::framework::stage_name(driver.driver.stage),
            driver.driver.name,
            crate::drivers::framework::class_name(driver.driver.class),
            crate::drivers::framework::bus_name(driver.driver.bus),
            crate::drivers::framework::state_name(driver.state)
        ));
    });
}

fn trace(debugger: &mut SerialDebugger) {
    crate::drivers::framework::trace(|event| {
        debugger.write_fmt(format_args!(
            "seq={}\t{}\tdev={}\tdrv={}\tstate={}\tresources={}\tirqs={}\tdmas={}\t{}\n",
            event.sequence,
            crate::drivers::framework::trace_op_name(event.operation),
            event.device,
            event.driver.unwrap_or(0),
            crate::drivers::framework::state_name(event.state),
            event.resources,
            event.irqs,
            event.dmas,
            if event.success { "ok" } else { "failed" },
        ));
    });
}

#[cfg(target_arch = "x86_64")]
fn ps2(debugger: &mut SerialDebugger, args: &[&str]) {
    if let Some(command) = args.first().copied() {
        let port = match command {
            "reset-keyboard" => crate::drivers::ps2::Port::Keyboard,
            "reset-mouse" => crate::drivers::ps2::Port::Mouse,
            _ => {
                debugger.println("ps2: use reset-keyboard, reset-mouse, or no argument");
                return;
            }
        };
        match crate::drivers::ps2::send_device_command(port, 0xff) {
            Ok(()) => debugger.println("ps2 reset command acknowledged"),
            Err(error) => debugger.write_fmt(format_args!("ps2: {error}\n")),
        }
        return;
    }

    let status = crate::drivers::ps2::status();
    let mut keyboard_bytes = 0;
    while crate::drivers::ps2::pop_keyboard().is_some() {
        keyboard_bytes += 1;
    }
    let mut mouse_bytes = 0;
    while crate::drivers::ps2::pop_mouse().is_some() {
        mouse_bytes += 1;
    }
    debugger.write_fmt(format_args!(
        "controller={} config=0x{:02x} keyboard={} mouse={} irq-registered={} irq-enabled={} drained-kbd={} drained-mouse={} dropped-kbd={} dropped-mouse={}\n",
        status.controller,
        status.config,
        status.keyboard_port,
        status.mouse_port,
        status.irq_registered,
        status.irq_enabled,
        keyboard_bytes,
        mouse_bytes,
        status.keyboard_dropped,
        status.mouse_dropped,
    ));
}

#[cfg(target_arch = "x86_64")]
fn keyboard(debugger: &mut SerialDebugger, args: &[&str]) {
    if let Some(command) = args.first().copied() {
        match command {
            "us" => crate::drivers::keyboard::set_layout(crate::drivers::keyboard::Layout::Us),
            "ru" => crate::drivers::keyboard::set_layout(crate::drivers::keyboard::Layout::Ru),
            "repeat-on" => crate::drivers::keyboard::set_repeat_policy(
                crate::drivers::keyboard::RepeatPolicy::Hardware,
            ),
            "repeat-off" => crate::drivers::keyboard::set_repeat_policy(
                crate::drivers::keyboard::RepeatPolicy::Disabled,
            ),
            _ => {
                debugger.println("keyboard: use us, ru, repeat-on, repeat-off, or no argument");
                return;
            }
        }
    }
    let status = crate::drivers::keyboard::status();
    debugger.write_fmt(format_args!(
        "layout={:?} repeat={:?} held={} parser-pending={} events-dropped={}\n",
        status.layout, status.repeat, status.held, status.parser_pending, status.events_dropped,
    ));
}

#[cfg(target_arch = "x86_64")]
fn input(debugger: &mut SerialDebugger) {
    let mut count = 0;
    while let Some(event) = crate::input::pop() {
        match event {
            crate::input::Event::Key(key) => debugger.write_fmt(format_args!(
                "key={:?} pressed={} repeat={} modifiers=0x{:02x} text={:?}\n",
                key.code,
                key.pressed,
                key.repeat,
                key.modifiers.bits(),
                key.text,
            )),
            crate::input::Event::Pointer(pointer) => debugger.write_fmt(format_args!(
                "pointer dx={} dy={} wheel={} buttons=0x{:02x} changed=0x{:02x}\n",
                pointer.dx,
                pointer.dy,
                pointer.wheel,
                pointer.buttons.bits(),
                pointer.changed.bits(),
            )),
        }
        count += 1;
    }
    if count == 0 {
        debugger.println("input queue empty");
    }
}

#[cfg(target_arch = "x86_64")]
fn mouse(debugger: &mut SerialDebugger) {
    let status = crate::drivers::mouse::status();
    debugger.write_fmt(format_args!(
        "id={} packet={} wheel={} extra-buttons={} buttons=0x{:02x} parser-pending={} events-dropped={}\n",
        status.id,
        status.packet_len,
        status.wheel,
        status.extra_buttons,
        status.buttons.bits(),
        status.parser_pending,
        status.events_dropped,
    ));
}

#[cfg(target_arch = "x86_64")]
fn usb(debugger: &mut SerialDebugger) {
    let Some(status) = crate::drivers::usb::xhci::status() else {
        debugger.println("xHCI unavailable");
        return;
    };
    debugger.write_fmt(format_args!(
        "kind={} state={:?} pci={:02x}:{:02x}.{} vendor=0x{:04x} device=0x{:04x} bar=0x{:x} slots={} ports={} connected={} interrupters={} addr64={} devices={} configured={}\n",
        crate::drivers::usb::xhci::kind_name(status.kind),
        status.state,
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
        debugger.write_fmt(format_args!(
            "device slot={} address={} speed={} vid=0x{:04x} pid=0x{:04x} configuration={} class={} hub-ports={} hid={}\n",
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
}

#[cfg(target_arch = "x86_64")]
fn audio(debugger: &mut SerialDebugger, args: &[&str]) {
    if args.first().copied() == Some("silence") {
        match crate::drivers::audio::submit_pcm(&[0; 8]) {
            Ok(()) => debugger.println("AC'97 silence submitted"),
            Err(error) => debugger.write_fmt(format_args!("AC'97 silence: {:?}\n", error)),
        }
    }
    let Some(status) = crate::drivers::audio::status() else {
        debugger.println("AC'97 unavailable; silent fallback active");
        return;
    };
    debugger.write_fmt(format_args!(
        "pci={:02x}:{:02x}.{} vendor=0x{:04x} device=0x{:04x} codec=0x{:04x}:0x{:04x} pcm={}Hz/{}ch/{}bit ring={}x{} running={} recoveries={}\n",
        status.bus,
        status.slot,
        status.function,
        status.vendor,
        status.device,
        status.codec_vendor1,
        status.codec_vendor2,
        status.format.sample_rate,
        status.format.channels,
        status.format.bits,
        status.ring_entries,
        status.buffer_bytes,
        status.running,
        status.recoveries,
    ));
}

#[cfg(target_arch = "x86_64")]
fn net(debugger: &mut SerialDebugger, args: &[&str]) {
    match args.first().copied() {
        Some("tx") => {
            let packet = [0u8; 60];
            match crate::drivers::network::transmit_packet(&packet) {
                Ok(()) => debugger.println("virtio-net test frame queued"),
                Err(error) => debugger.write_fmt(format_args!("virtio-net tx: {:?}\n", error)),
            }
        }
        Some("rx") => {
            let mut packet = [0u8; 1514];
            match crate::drivers::network::receive_packet(&mut packet) {
                Ok(length) => {
                    debugger.write_fmt(format_args!("virtio-net frame received bytes={}\n", length))
                }
                Err(error) => debugger.write_fmt(format_args!("virtio-net rx: {:?}\n", error)),
            }
        }
        Some("ping") => {
            let Some(address) = args
                .get(1)
                .and_then(|value| crate::net::Ipv4Addr::parse(value))
            else {
                debugger.println("net ping: use ping <ipv4>");
                return;
            };
            match crate::net::ping(address) {
                Ok(()) => debugger.println("ICMP echo request queued"),
                Err(error) => debugger.write_fmt(format_args!("net ping: {:?}\n", error)),
            }
        }
        Some("udp") if args.get(1).copied() == Some("recv") => {
            let mut payload = [0u8; 512];
            match crate::net::udp_receive(&mut payload) {
                Ok((source, port, length)) => debugger.write_fmt(format_args!(
                    "UDP datagram received from={}.{}.{}.{}:{} bytes={} data={:?}\n",
                    source.0[0],
                    source.0[1],
                    source.0[2],
                    source.0[3],
                    port,
                    length,
                    core::str::from_utf8(&payload[..length]).unwrap_or("<binary>"),
                )),
                Err(error) => debugger.write_fmt(format_args!("net udp recv: {:?}\n", error)),
            }
        }
        Some("udp") => {
            let Some(address) = args
                .get(1)
                .and_then(|value| crate::net::Ipv4Addr::parse(value))
            else {
                debugger.println("net udp: use udp <ipv4> <port> [payload]");
                return;
            };
            let Some(port) = args.get(2).and_then(|value| value.parse::<u16>().ok()) else {
                debugger.println("net udp: use udp <ipv4> <port> [payload]");
                return;
            };
            let payload = args.get(3).copied().unwrap_or("").as_bytes();
            match crate::net::udp_send(address, port, payload) {
                Ok(()) => debugger.write_fmt(format_args!(
                    "UDP datagram queued bytes={}\n",
                    payload.len()
                )),
                Err(error) => debugger.write_fmt(format_args!("net udp: {:?}\n", error)),
            }
        }
        Some("dns") => {
            let Some(name) = args.get(1).copied() else {
                debugger.println("net dns: use dns <name>");
                return;
            };
            match crate::net::dns_query(name) {
                Ok(()) => debugger.write_fmt(format_args!("DNS query queued name={name}\n")),
                Err(error) => debugger.write_fmt(format_args!("net dns: {:?}\n", error)),
            }
        }
        Some("tcp") => match args.get(1).copied() {
            Some("connect") => {
                let Some(address) = args
                    .get(2)
                    .and_then(|value| crate::net::Ipv4Addr::parse(value))
                else {
                    debugger.println("net tcp connect: use tcp connect <ipv4> <port>");
                    return;
                };
                let Some(port) = args.get(3).and_then(|value| value.parse::<u16>().ok()) else {
                    debugger.println("net tcp connect: use tcp connect <ipv4> <port>");
                    return;
                };
                match crate::net::tcp_connect(address, port) {
                    Ok(()) => debugger.println("TCP SYN queued"),
                    Err(error) => {
                        debugger.write_fmt(format_args!("net tcp connect: {:?}\n", error))
                    }
                }
            }
            Some("send") => {
                let Some(payload) = args.get(2).copied() else {
                    debugger.println("net tcp send: use tcp send <payload>");
                    return;
                };
                match crate::net::tcp_send(payload.as_bytes()) {
                    Ok(()) => debugger
                        .write_fmt(format_args!("TCP payload queued bytes={}\n", payload.len())),
                    Err(error) => debugger.write_fmt(format_args!("net tcp send: {:?}\n", error)),
                }
            }
            Some("recv") => {
                let mut payload = [0u8; 512];
                match crate::net::tcp_receive(&mut payload) {
                    Ok(length) => debugger.write_fmt(format_args!(
                        "TCP payload received bytes={} data={:?}\n",
                        length,
                        core::str::from_utf8(&payload[..length]).unwrap_or("<binary>"),
                    )),
                    Err(error) => debugger.write_fmt(format_args!("net tcp recv: {:?}\n", error)),
                }
            }
            _ => {
                debugger.println("net tcp: use connect <ipv4> <port>, send <payload>, or recv");
                return;
            }
        },
        Some(command) => debugger.write_fmt(format_args!(
            "net: unknown command {command}; use ping, udp, dns, tcp, tx, rx, or no argument\n"
        )),
        None => {}
    }
    let Some(status) = crate::drivers::network::status() else {
        debugger.println("virtio-net unavailable; networking deferred");
        return;
    };
    debugger.write_fmt(format_args!(
        "pci={:02x}:{:02x}.{} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} link={} irq={} interrupts={} rxq={} txq={} rx-pending={} rx={} tx={} dropped={} busy={}\n",
        status.bus,
        status.slot,
        status.function,
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
        status.rx_pending,
        status.rx_packets,
        status.tx_packets,
        status.rx_dropped,
        status.tx_busy,
    ));
    let Some(stack) = crate::net::status() else {
        debugger.println("network stack unavailable; networking deferred");
        return;
    };
    debugger.write_fmt(format_args!(
        "stack link={} configured={} ip={}.{}.{}.{} gateway={}.{}.{}.{} dns={}.{}.{}.{} arp={} rx={} dropped={} tx={} dhcp={:?} dns-pending={} dns-result={:?} tcp={:?} tcp-rx={}\n",
        stack.link_up,
        stack.configured,
        stack.ip.0[0],
        stack.ip.0[1],
        stack.ip.0[2],
        stack.ip.0[3],
        stack.gateway.0[0],
        stack.gateway.0[1],
        stack.gateway.0[2],
        stack.gateway.0[3],
        stack.dns.0[0],
        stack.dns.0[1],
        stack.dns.0[2],
        stack.dns.0[3],
        stack.arp_entries,
        stack.rx_frames,
        stack.rx_dropped,
        stack.tx_frames,
        stack.dhcp,
        stack.dns_pending,
        stack.dns_result,
        stack.tcp,
        stack.tcp_rx_bytes,
    ));
}

fn block(debugger: &mut SerialDebugger, args: &[&str]) {
    match args.first().copied() {
        Some("ro") => crate::drivers::block::set_read_only(true),
        Some("rw") => crate::drivers::block::set_read_only(false),
        Some("flush") => match crate::drivers::block::flush_cache() {
            Ok(()) => debugger.println("block cache flushed"),
            Err(error) => debugger.write_fmt(format_args!("block flush: {:?}\n", error)),
        },
        Some("write-through") => match crate::drivers::block::set_cache_mode(
            crate::drivers::block::CacheMode::WriteThrough,
        ) {
            Ok(()) => debugger.println("block cache=write-through"),
            Err(error) => debugger.write_fmt(format_args!("block cache: {:?}\n", error)),
        },
        Some("write-back") => {
            match crate::drivers::block::set_cache_mode(crate::drivers::block::CacheMode::WriteBack)
            {
                Ok(()) => debugger.println("block cache=write-back"),
                Err(error) => debugger.write_fmt(format_args!("block cache: {:?}\n", error)),
            }
        }
        Some(command) => debugger.write_fmt(format_args!("block: unknown command {command}\n")),
        None => {}
    }
    let stats = crate::drivers::block::stats();
    debugger.write_fmt(format_args!(
        "sector={} sectors={} queue={}/{} completed={} timeouts={} partitions={} readonly={} cache={}\n",
        stats.geometry.sector_size,
        stats.geometry.sectors,
        stats.in_flight,
        stats.queue_capacity,
        stats.completed,
        stats.timeouts,
        stats.partition_count,
        stats.geometry.read_only,
        stats.cache_mode.name(),
    ));
    let mut partitions = [crate::drivers::block::Partition {
        index: 0,
        type_code: 0,
        start_lba: 0,
        sectors: 0,
    }; 4];
    let partition_count = crate::drivers::block::partitions(&mut partitions);
    for partition in &partitions[..partition_count] {
        debugger.write_fmt(format_args!(
            "partition{} type=0x{:02x} start={} sectors={}\n",
            partition.index, partition.type_code, partition.start_lba, partition.sectors,
        ));
    }
}

fn vfs(debugger: &mut SerialDebugger, args: &[&str]) {
    match args.first().copied() {
        Some("mount") => {
            let Some(target) = args.get(1).copied() else {
                debugger.println("vfs mount: missing target");
                return;
            };
            match crate::vfs::mount(
                crate::vfs::MountSource::Ramfs,
                target,
                crate::vfs::MountFlags::defaults(),
            ) {
                Ok(id) => debugger.write_fmt(format_args!(
                    "mounted id={} source=ramfs target={}\n",
                    id.raw(),
                    target
                )),
                Err(error) => debugger.write_fmt(format_args!("vfs mount: {:?}\n", error)),
            }
        }
        Some("umount") => {
            let Some(id) = args.get(1).and_then(|value| parse_u8(value)) else {
                debugger.println("vfs umount: expected numeric mount id");
                return;
            };
            let Some(id) = crate::vfs::MountId::from_raw(id) else {
                debugger.println("vfs umount: invalid mount id");
                return;
            };
            match crate::vfs::unmount_mount(id) {
                Ok(()) => debugger.write_fmt(format_args!("unmounted id={}\n", id.raw())),
                Err(error) => debugger.write_fmt(format_args!("vfs umount: {:?}\n", error)),
            }
        }
        Some("lookup") => {
            let Some(path) = args.get(1).copied() else {
                debugger.println("vfs lookup: missing path");
                return;
            };
            match crate::vfs::lookup(path) {
                Ok(dentry) => {
                    let value = dentry.dentry();
                    let _ = crate::vfs::release_dentry(dentry);
                    debugger.write_fmt(format_args!(
                        "lookup path={} mount={} inode={}\n",
                        path,
                        value.mount.raw(),
                        value.inode
                    ));
                }
                Err(error) => debugger.write_fmt(format_args!("vfs lookup: {:?}\n", error)),
            }
        }
        Some("namespace") => match crate::vfs::create_namespace() {
            Ok(namespace) => {
                debugger.write_fmt(format_args!("namespace created {:?}\n", namespace))
            }
            Err(error) => debugger.write_fmt(format_args!("vfs namespace: {:?}\n", error)),
        },
        Some(command) => debugger.write_fmt(format_args!("vfs: unknown action {command}\n")),
        None => {}
    }
    let stats = crate::vfs::stats();
    debugger.write_fmt(format_args!(
        "mounts={} directories={} files={} bytes={} open={}\n",
        stats.mounts, stats.directories, stats.files, stats.bytes, stats.open_handles,
    ));
    crate::vfs::list_mounts(|info| {
        debugger.write_fmt(format_args!(
            "mount id={} ns={:?} parent={:?} source={:?} propagation={:?} ro={} noexec={} nosuid={} nodev={}\n",
            info.id.raw(),
            info.namespace,
            info.parent,
            info.source,
            info.propagation,
            info.flags.read_only,
            info.flags.no_exec,
            info.flags.no_suid,
            info.flags.no_dev,
        ));
    });
}

fn parse_u8(value: &str) -> Option<u8> {
    let mut result = 0u8;
    for byte in value.bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add(byte - b'0')?;
    }
    Some(result)
}

fn ls(debugger: &mut SerialDebugger) {
    crate::vfs::list(|path, len| {
        debugger.write_fmt(format_args!("{}\t{} bytes\n", path, len));
    });
}

fn cat(debugger: &mut SerialDebugger, args: &[&str]) {
    let Some(path) = args.first().copied() else {
        debugger.println("cat: missing path");
        return;
    };
    let mut out = [0u8; 255];
    let Some(len) = crate::vfs::read(path, &mut out) else {
        debugger.write_fmt(format_args!("cat: {}: not found\n", path));
        return;
    };
    debugger.write(core::str::from_utf8(&out[..len]).unwrap_or("<binary>"));
    if len == 0 || out[len - 1] != b'\n' {
        debugger.println("");
    }
}

fn write_file(debugger: &mut SerialDebugger, args: &[&str]) {
    let Some(path) = args.first().copied() else {
        debugger.println("write: missing path");
        return;
    };
    let mut data = [0u8; 255];
    let mut len = 0;
    for (i, arg) in args[1..].iter().enumerate() {
        if i != 0 {
            if len == data.len() {
                break;
            }
            data[len] = b' ';
            len += 1;
        }
        for byte in arg.bytes() {
            if len == data.len() {
                break;
            }
            data[len] = byte;
            len += 1;
        }
    }
    if !crate::vfs::write(path, &data[..len]) {
        debugger.write_fmt(format_args!("write: {}: failed\n", path));
        return;
    }
    debugger.write_fmt(format_args!("{} bytes written\n", len));
}
