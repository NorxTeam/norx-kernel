const CONFIG_ADDRESS: u16 = 0x0cf8;
const CONFIG_DATA: u16 = 0x0cfc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Address {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    Pio(u16),
    Mmio(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Device {
    pub address: Address,
    pub vendor: u16,
    pub device: u16,
}

impl Device {
    pub fn bar(self, index: u8) -> Option<Bar> {
        if index > 5 {
            return None;
        }
        let offset = 0x10 + index * 4;
        let lower = self.read(offset);
        if lower == 0 || lower == u32::MAX {
            return None;
        }
        let upper = (lower & 0x6 == 0x4).then(|| self.read(offset + 4));
        decode_bar(lower, upper)
    }

    pub fn enable(self, io: bool, memory: bool, bus_master: bool) -> bool {
        let mut command = self.read(0x04) as u16;
        if io {
            command |= 1;
        }
        if memory {
            command |= 1 << 1;
        }
        if bus_master {
            command |= 1 << 2;
        }
        self.write(0x04, command as u32);
        self.read(0x04) as u16 & command == command
    }

    pub fn irq_line(self) -> u8 {
        self.read(0x3c) as u8
    }

    pub fn read(self, offset: u8) -> u32 {
        let config = 0x8000_0000u32
            | (self.address.bus as u32) << 16
            | (self.address.slot as u32) << 11
            | (self.address.function as u32) << 8
            | (offset as u32 & 0xfc);
        crate::arch::port_write_u32(CONFIG_ADDRESS, config);
        crate::arch::port_read_u32(CONFIG_DATA)
    }

    fn write(self, offset: u8, value: u32) {
        let config = 0x8000_0000u32
            | (self.address.bus as u32) << 16
            | (self.address.slot as u32) << 11
            | (self.address.function as u32) << 8
            | (offset as u32 & 0xfc);
        crate::arch::port_write_u32(CONFIG_ADDRESS, config);
        crate::arch::port_write_u32(CONFIG_DATA, value);
    }
}

pub fn find_class(class_code: u32) -> Option<Device> {
    find_class_vendor(class_code, None)
}

pub fn find_class_vendor(class_code: u32, vendor: Option<u16>) -> Option<Device> {
    for slot in 0..32 {
        for function in 0..8 {
            let device = Device {
                address: Address {
                    bus: 0,
                    slot,
                    function,
                },
                vendor: 0,
                device: 0,
            };
            let identity = device.read(0x00);
            if identity == u32::MAX {
                if function == 0 {
                    break;
                }
                continue;
            }
            let candidate = Device::from_identity(device, identity);
            if vendor.is_none_or(|expected| expected == candidate.vendor)
                && class_code_matches(candidate.read(0x08), class_code)
            {
                return Some(candidate);
            }
        }
    }
    None
}

impl Device {
    fn from_identity(device: Self, identity: u32) -> Self {
        Self {
            vendor: identity as u16,
            device: (identity >> 16) as u16,
            ..device
        }
    }
}

pub fn contract_self_check() {
    let address = Address {
        bus: 0,
        slot: 3,
        function: 0,
    };
    assert_eq!(address.slot, 3);
    assert_eq!(Bar::Pio(0x6000), Bar::Pio(0x6000));
    assert!(class_code_matches(0x0100_0000, 0x0001_0000));
    assert!(!class_code_matches(0x0200_0000, 0x0001_0000));
    assert!(!class_code_matches(0x0100_0100, 0x0001_0000));
    assert_eq!(decode_bar(0xfebf_8000, None), Some(Bar::Mmio(0xfebf_8000)));
    assert_eq!(
        decode_bar(0x0000_0004, Some(0x0000_0001)),
        Some(Bar::Mmio(0x0000_0001_0000_0000))
    );
    assert_eq!(decode_bar(0x0000_0001, None), Some(Bar::Pio(0)));
    assert_eq!(decode_bar(0, None), None);
}

const fn class_code_matches(config_class: u32, expected: u32) -> bool {
    (config_class >> 8) & 0x00ff_ffff == expected
}

fn decode_bar(lower: u32, upper: Option<u32>) -> Option<Bar> {
    if lower == 0 || lower == u32::MAX {
        return None;
    }
    if lower & 1 != 0 {
        return Some(Bar::Pio((lower & !3) as u16));
    }
    let base = if lower & 0x6 == 0x4 {
        (upper? as u64) << 32 | (lower as u64 & 0xffff_fff0)
    } else {
        lower as u64 & 0xffff_fff0
    };
    (base != 0).then_some(Bar::Mmio(base))
}
