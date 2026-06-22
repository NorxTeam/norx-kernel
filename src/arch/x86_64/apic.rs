use core::arch::asm;

#[derive(Clone, Copy)]
pub struct Status {
    pub present: bool,
    pub x2apic: bool,
    pub enabled: bool,
    pub software_enabled: bool,
    pub base: u64,
    pub id: u32,
    pub version: u32,
}

pub fn init() -> Status {
    let mut current = status();
    if current.present && !current.enabled {
        unsafe {
            let msr = rdmsr(0x1b) | (1 << 11);
            wrmsr(0x1b, msr);
        }
        current = status();
    }
    if current.enabled && !current.x2apic {
        unsafe {
            let base = current.base as *mut u32;
            let spurious = read(base, 0x0f0) | (1 << 8) | 0xff;
            write(base, 0x0f0, spurious);
        }
        current = status();
    }
    current
}

pub fn status() -> Status {
    let features = cpuid(1, 0);
    let present = features.edx & (1 << 9) != 0;
    let x2apic = features.ecx & (1 << 21) != 0;
    let msr = if present { unsafe { rdmsr(0x1b) } } else { 0 };
    let enabled = present && msr & (1 << 11) != 0;
    let base = msr & 0x000f_ffff_ffff_f000;
    let (software_enabled, id, version) = if enabled && !x2apic {
        unsafe {
            let mmio = base as *mut u32;
            let spurious = read(mmio, 0x0f0);
            let id = read(mmio, 0x020) >> 24;
            let version = read(mmio, 0x030) & 0xff;
            (spurious & (1 << 8) != 0, id, version)
        }
    } else {
        (false, 0, 0)
    };

    Status {
        present,
        x2apic,
        enabled,
        software_enabled,
        base,
        id,
        version,
    }
}

unsafe fn read(base: *mut u32, offset: usize) -> u32 {
    base.byte_add(offset).read_volatile()
}

unsafe fn write(base: *mut u32, offset: usize, value: u32) {
    base.byte_add(offset).write_volatile(value);
}

#[derive(Clone, Copy)]
struct Cpuid {
    ecx: u32,
    edx: u32,
}

fn cpuid(leaf: u32, subleaf: u32) -> Cpuid {
    let ecx: u32;
    let edx: u32;
    unsafe {
        asm!(
            "push rbx",
            "cpuid",
            "pop rbx",
            inout("eax") leaf => _,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
            options(preserves_flags)
        );
    }
    Cpuid { ecx, edx }
}

unsafe fn rdmsr(msr: u32) -> u64 {
    let high: u32;
    let low: u32;
    asm!(
        "rdmsr",
        in("ecx") msr,
        out("edx") high,
        out("eax") low,
        options(nomem, nostack, preserves_flags)
    );
    ((high as u64) << 32) | low as u64
}

unsafe fn wrmsr(msr: u32, value: u64) {
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("edx") (value >> 32) as u32,
        in("eax") value as u32,
        options(nomem, nostack, preserves_flags)
    );
}
