use core::fmt::{self, Write};

const LINE_MAX: usize = 128;
const ARG_MAX: usize = 8;

pub fn run() -> ! {
    let mut debugger = SerialDebugger::new();
    debugger.banner();

    loop {
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
                "commands: help time sched irq hw mem paging drivers uname vfs ls cat write crash halt",
            ),
            "time" => show_time(self),
            "sched" => sched(self),
            "irq" => irq(self),
            "hw" => hw(self),
            "mem" => mem(self),
            "paging" => paging(self),
            "drivers" => drivers(self),
            "uname" => self.write_fmt(format_args!("Norx {} grub\n", crate::arch::NAME)),
            "vfs" => vfs(self),
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
        "timer={} spurious={} exceptions={}\n",
        stats.timer, stats.spurious, stats.exceptions,
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
            "{}\t{}\t{}\n",
            driver.name,
            crate::drivers::framework::class_name(driver.class),
            crate::drivers::framework::state_name(driver.state)
        ));
    });
}

fn vfs(debugger: &mut SerialDebugger) {
    let stats = crate::vfs::stats();
    debugger.write_fmt(format_args!(
        "mounts={} files={} bytes={}\n",
        stats.mounts, stats.files, stats.bytes,
    ));
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
