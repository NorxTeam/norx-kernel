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
        let Some(mmio) = (unsafe { crate::io::MmioRegion::new(current.base as usize, 0x400) })
        else {
            return current;
        };
        let Some(spurious) = mmio.read_u32_le(0x0f0) else {
            return current;
        };
        if !mmio.write_u32_le(0x0f0, spurious | (1 << 8) | 0xff) {
            return current;
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
        let Some(mmio) = (unsafe { crate::io::MmioRegion::new(base as usize, 0x400) }) else {
            return Status {
                present,
                x2apic,
                enabled,
                software_enabled: false,
                base,
                id: 0,
                version: 0,
            };
        };
        let Some(spurious) = mmio.read_u32_le(0x0f0) else {
            return Status {
                present,
                x2apic,
                enabled,
                software_enabled: false,
                base,
                id: 0,
                version: 0,
            };
        };
        let Some(id_value) = mmio.read_u32_le(0x020) else {
            return Status {
                present,
                x2apic,
                enabled,
                software_enabled: false,
                base,
                id: 0,
                version: 0,
            };
        };
        let Some(version_value) = mmio.read_u32_le(0x030) else {
            return Status {
                present,
                x2apic,
                enabled,
                software_enabled: false,
                base,
                id: 0,
                version: 0,
            };
        };
        (
            spurious & (1 << 8) != 0,
            id_value >> 24,
            version_value & 0xff,
        )
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
