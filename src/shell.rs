use core::fmt::{self, Write};

const LINE_MAX: usize = 128;
const ARG_MAX: usize = 8;
const HISTORY_MAX: usize = 8;

pub fn run() -> ! {
    crate::log::clear_screen();

    let mut user = Session::new(Target::User, "kernel");
    let mut serial = Session::new(Target::Serial, "serial");
    user.banner();
    serial.banner();

    loop {
        if !crate::timer::hardware_ticks() && crate::time::poll_scheduler_tick() {
            crate::sched::on_timer_tick();
        }
        if let Some(key) = crate::input::poll_user() {
            user.key(key);
        }
        if let Some(key) = crate::input::poll_serial() {
            serial.key(key);
        }
    }
}

#[derive(Clone, Copy)]
enum Target {
    User,
    Serial,
}

struct Session {
    target: Target,
    owner: &'static str,
    cwd: &'static str,
    line: [u8; LINE_MAX],
    len: usize,
    cursor: usize,
    drawn: usize,
    history_offset: usize,
    history: History,
}

impl Session {
    const fn new(target: Target, owner: &'static str) -> Self {
        Self {
            target,
            owner,
            cwd: "/",
            line: [0; LINE_MAX],
            len: 0,
            cursor: 0,
            drawn: 0,
            history_offset: 0,
            history: History::new(),
        }
    }

    fn banner(&mut self) {
        self.println("Norx Shell");
        self.println("sessions: user framebuffer + serial console");
        self.println(
            "builtins: help clear echo time ticks sched irq input hw mem paging dmaptest vm block vfs ls cat write uname drivers lazytest crash halt",
        );
        self.prompt();
    }

    fn key(&mut self, key: crate::input::Key) {
        match key {
            crate::input::Key::Enter => {
                self.println("");
                self.history.push(&self.line, self.len);
                let mut command = [0u8; LINE_MAX];
                command[..self.len].copy_from_slice(&self.line[..self.len]);
                let command_len = self.len;
                exec(
                    self,
                    core::str::from_utf8(&command[..command_len]).unwrap_or(""),
                );
                self.len = 0;
                self.cursor = 0;
                self.drawn = 0;
                self.history_offset = 0;
                self.prompt();
            }
            crate::input::Key::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.len -= 1;
                    for i in self.cursor..self.len {
                        self.line[i] = self.line[i + 1];
                    }
                    self.redraw();
                }
            }
            crate::input::Key::Delete => {
                if self.cursor < self.len {
                    self.len -= 1;
                    for i in self.cursor..self.len {
                        self.line[i] = self.line[i + 1];
                    }
                    self.redraw();
                }
            }
            crate::input::Key::Left => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.write("\x1b[D");
                }
            }
            crate::input::Key::Right => {
                if self.cursor < self.len {
                    self.cursor += 1;
                    self.write("\x1b[C");
                }
            }
            crate::input::Key::Home => {
                while self.cursor > 0 {
                    self.cursor -= 1;
                    self.write("\x1b[D");
                }
            }
            crate::input::Key::End => {
                while self.cursor < self.len {
                    self.cursor += 1;
                    self.write("\x1b[C");
                }
            }
            crate::input::Key::Up => {
                if self.history_offset < self.history.len {
                    self.history_offset += 1;
                    self.len = self
                        .history
                        .get(self.history_offset, &mut self.line)
                        .unwrap_or(0);
                    self.cursor = self.len;
                    self.redraw();
                }
            }
            crate::input::Key::Down => {
                if self.history_offset > 1 {
                    self.history_offset -= 1;
                    self.len = self
                        .history
                        .get(self.history_offset, &mut self.line)
                        .unwrap_or(0);
                } else {
                    self.history_offset = 0;
                    self.len = 0;
                }
                self.cursor = self.len;
                self.redraw();
            }
            crate::input::Key::Char(byte) if byte.is_ascii_graphic() || byte == b' ' => {
                if self.len == LINE_MAX - 1 {
                    return;
                }
                for i in (self.cursor..self.len).rev() {
                    self.line[i + 1] = self.line[i];
                }
                self.line[self.cursor] = byte;
                self.len += 1;
                self.cursor += 1;
                self.history_offset = 0;
                self.redraw();
            }
            _ => {}
        }
    }

    fn prompt(&mut self) {
        self.write("\x1b[96m");
        self.write("(");
        self.write(self.owner);
        self.write(") ");
        self.write("\x1b[0m");
        self.write(self.cwd);
        self.write(" % ");
    }

    fn redraw(&mut self) {
        self.write("\r");
        self.prompt();
        let mut line = [0u8; LINE_MAX];
        line[..self.len].copy_from_slice(&self.line[..self.len]);
        self.write(core::str::from_utf8(&line[..self.len]).unwrap_or(""));
        for _ in self.len..=self.drawn {
            self.write(" ");
        }
        self.drawn = self.len;
        self.write("\r");
        self.prompt();
        for _ in 0..self.cursor {
            self.write("\x1b[C");
        }
    }

    fn println(&mut self, s: &str) {
        self.write(s);
        self.write("\n");
    }

    fn write(&mut self, s: &str) {
        match self.target {
            Target::User => crate::log::screen_write_str(s),
            Target::Serial => crate::drivers::serial::write_str(s),
        }
    }

    fn write_fmt(&mut self, args: fmt::Arguments) {
        let _ = SessionWriter(self).write_fmt(args);
    }
}

struct SessionWriter<'a>(&'a mut Session);

impl Write for SessionWriter<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0.write(s);
        Ok(())
    }
}

struct History {
    lines: [[u8; LINE_MAX]; HISTORY_MAX],
    lens: [usize; HISTORY_MAX],
    len: usize,
    next: usize,
}

impl History {
    const fn new() -> Self {
        Self {
            lines: [[0; LINE_MAX]; HISTORY_MAX],
            lens: [0; HISTORY_MAX],
            len: 0,
            next: 0,
        }
    }

    fn push(&mut self, line: &[u8; LINE_MAX], len: usize) {
        if len == 0 {
            return;
        }
        self.lines[self.next][..len].copy_from_slice(&line[..len]);
        self.lens[self.next] = len;
        self.next = (self.next + 1) % HISTORY_MAX;
        self.len = (self.len + 1).min(HISTORY_MAX);
    }

    fn get(&self, offset: usize, out: &mut [u8; LINE_MAX]) -> Option<usize> {
        if offset == 0 || offset > self.len {
            return None;
        }
        let index = (self.next + HISTORY_MAX - offset) % HISTORY_MAX;
        let len = self.lens[index];
        out[..len].copy_from_slice(&self.lines[index][..len]);
        Some(len)
    }
}

fn exec(session: &mut Session, line: &str) {
    let mut args = [""; ARG_MAX];
    let argc = split(line, &mut args);
    if argc == 0 {
        return;
    }

    match args[0] {
        "help" => session.println(
            "builtins: help clear echo time ticks sched irq input hw mem paging dmaptest vm block vfs ls cat write uname drivers lazytest crash halt",
        ),
        "clear" => {
            if matches!(session.target, Target::User) {
                crate::log::clear_screen();
            }
        }
        "echo" => {
            for (i, arg) in args[1..argc].iter().enumerate() {
                if i != 0 {
                    session.write(" ");
                }
                session.write(arg);
            }
            session.println("");
        }
        "time" => show_time(session),
        "ticks" => session.write_fmt(format_args!("{}\n", crate::time::ticks())),
        "sched" => sched(session),
        "irq" => irq(session),
        "input" => input(session),
        "hw" => hw(session),
        "mem" => mem(session),
        "paging" => paging(session),
        "dmaptest" => dmaptest(session),
        "vm" => vm(session),
        "block" => block(session, &args[1..argc]),
        "vfs" => vfs(session),
        "ls" => ls(session),
        "cat" => cat(session, &args[1..argc]),
        "write" => write_file(session, &args[1..argc]),
        "uname" => session.write_fmt(format_args!("Norx {} grub\n", crate::arch::NAME)),
        "drivers" => drivers(session),
        "lazytest" => lazytest(session),
        "crash" => crate::crash::fatal(crate::error::KernelError {
            kind: crate::error::ErrorKind::Panic,
            code: 0xdead,
            message: "manual crash",
            detail: "requested by shell command",
            arg0: 0,
            arg1: 0,
        }),
        "halt" => crate::arch::halt(),
        cmd => session.write_fmt(format_args!("unknown command: {}\n", cmd)),
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

fn show_time(session: &mut Session) {
    if let Some(ticks) = crate::time::boot_time() {
        session.write_fmt(format_args!(
            "boot ticks={} now={}\n",
            ticks,
            crate::time::ticks()
        ));
    } else {
        session.println("time unavailable");
    }
}

fn drivers(session: &mut Session) {
    crate::drivers::framework::list(|driver| {
        session.write_fmt(format_args!(
            "{}\t{}\t{}\n",
            driver.name,
            crate::drivers::framework::class_name(driver.class),
            crate::drivers::framework::state_name(driver.state)
        ));
    });
}

fn sched(session: &mut Session) {
    let status = crate::sched::status();
    session.write_fmt(format_args!(
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

fn irq(session: &mut Session) {
    let stats = crate::irq::stats();
    session.write_fmt(format_args!(
        "timer={} keyboard={} spurious={} exceptions={}\n",
        stats.timer, stats.keyboard, stats.spurious, stats.exceptions,
    ));
}

fn input(session: &mut Session) {
    #[cfg(target_arch = "x86_64")]
    session.write_fmt(format_args!(
        "keyboard=pci-legacy-ps2 mode=irq pending={} dropped={}\n",
        crate::drivers::keyboard::pending(),
        crate::drivers::keyboard::dropped(),
    ));

    #[cfg(not(target_arch = "x86_64"))]
    session.println("keyboard unavailable");
}

fn hw(session: &mut Session) {
    #[cfg(target_arch = "x86_64")]
    {
        let apic = crate::arch::apic::status();
        session.write_fmt(format_args!(
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
    session.write_fmt(format_args!(
        "arch=aarch64 gic=false irq-timer={} lazy-pages={}\n",
        crate::timer::hardware_ticks(),
        crate::arch::supports_lazy_pages(),
    ));
}

fn mem(session: &mut Session) {
    let stats = crate::memory::stats();
    session.write_fmt(format_args!(
        "ranges={} usable={}KiB allocated_frames={} next=0x{:x}\n",
        stats.ranges,
        stats.usable_pages * 4,
        stats.allocated_frames,
        stats.next_frame,
    ));
}

fn vm(session: &mut Session) {
    let stats = crate::vm::stats();
    session.write_fmt(format_args!(
        "lazy={} base=0x{:x} pages={} bytes={}KiB\n",
        stats.lazy_supported,
        stats.lazy_base,
        stats.lazy_pages,
        stats.lazy_pages * 4,
    ));
}

fn paging(session: &mut Session) {
    let stats = crate::paging::stats();
    session.write_fmt(format_args!(
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
        session.write_fmt(format_args!(
            "aarch64 ttbr0=0x{:x} ttbr1=0x{:x} tcr=0x{:x} mair=0x{:x} sctlr=0x{:x} tables={}\n",
            reg.ttbr0, reg.ttbr1, reg.tcr, reg.mair, reg.sctlr, reg.tables_used,
        ));
    }
}

fn dmaptest(session: &mut Session) {
    #[cfg(target_arch = "x86_64")]
    {
        match crate::arch::paging::direct_map_ptr(0x1000) {
            Some(ptr) => {
                let value = unsafe { ptr.read_volatile() };
                session.write_fmt(format_args!(
                    "direct-map read phys=0x1000 byte=0x{:02x}\n",
                    value
                ));
            }
            None => session.println("direct-map unavailable"),
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    session.println("direct-map unavailable");
}

fn block(session: &mut Session, args: &[&str]) {
    let dev = crate::drivers::block::device();
    if args.first().copied() == Some("test") {
        let mut sector = [0u8; 512];
        for (i, byte) in sector.iter_mut().enumerate() {
            *byte = 255 - (i & 0xff) as u8;
        }
        let wrote = crate::drivers::block::write_sector(1, &sector);
        let mut readback = [0u8; 512];
        let read = crate::drivers::block::read_sector(1, &mut readback);
        session.write_fmt(format_args!(
            "{} test write={} read={} first=0x{:02x} last=0x{:02x}\n",
            dev.name, wrote, read, readback[0], readback[511],
        ));
        return;
    }

    let mut sector = [0u8; 512];
    let _ = crate::drivers::block::read_sector(0, &mut sector);
    session.write_fmt(format_args!(
        "{} sectors={} sector={} first=0x{:02x} second=0x{:02x}\n",
        dev.name, dev.sectors, dev.sector_size, sector[0], sector[1],
    ));
}

fn vfs(session: &mut Session) {
    let stats = crate::vfs::stats();
    session.write_fmt(format_args!(
        "mounts={} files={} bytes={}\n",
        stats.mounts, stats.files, stats.bytes,
    ));
}

fn ls(session: &mut Session) {
    crate::vfs::list(|path, len| {
        session.write_fmt(format_args!("{}\t{} bytes\n", path, len));
    });
}

fn cat(session: &mut Session, args: &[&str]) {
    let Some(path) = args.first().copied() else {
        session.println("cat: missing path");
        return;
    };
    let mut out = [0u8; 255];
    let Some(len) = crate::vfs::read(path, &mut out) else {
        session.write_fmt(format_args!("cat: {}: not found\n", path));
        return;
    };
    session.write(core::str::from_utf8(&out[..len]).unwrap_or("<binary>"));
    if len == 0 || out[len - 1] != b'\n' {
        session.println("");
    }
}

fn write_file(session: &mut Session, args: &[&str]) {
    let Some(path) = args.first().copied() else {
        session.println("write: missing path");
        return;
    };
    let mut data = [0u8; 511];
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
        session.write_fmt(format_args!("write: {}: failed\n", path));
        return;
    }
    session.write_fmt(format_args!("{} bytes written\n", len));
}

fn lazytest(session: &mut Session) {
    match crate::vm::lazy_probe() {
        Some(value) => session.write_fmt(format_args!(
            "lazy page 0x{:x} value=0x{:02x}\n",
            crate::vm::LAZY_BASE,
            value
        )),
        None => session.println("lazy allocation unavailable"),
    }
}
