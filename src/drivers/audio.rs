use crate::drivers::framework::{DmaBuffer, DmaDirection};
use crate::drivers::pci;
use crate::io::PioRegion;

const AC97_CLASS: u32 = 0x0004_0100;
const NAM_SIZE: u16 = 0x0400;
const NABM_SIZE: u16 = 0x0100;
const PCM_RATE: u16 = 48_000;
const PCM_BYTES: usize = 4096;
const RING_ENTRIES: usize = 4;

const NAM_RESET: usize = 0x00;
const NAM_MASTER_VOLUME: usize = 0x02;
const NAM_PCM_OUT_VOLUME: usize = 0x18;
const NAM_EXTENDED_AUDIO_ID: usize = 0x28;
const NAM_EXTENDED_AUDIO_CONTROL: usize = 0x2a;
const NAM_FRONT_DAC_RATE: usize = 0x2c;
const NAM_VENDOR_ID1: usize = 0x7c;
const NAM_VENDOR_ID2: usize = 0x7e;

const PO_BDBAR: usize = 0x00;
const PO_LVI: usize = 0x05;
const PO_STATUS: usize = 0x06;
const PO_CONTROL: usize = 0x0b;
const GLOBAL_STATUS: usize = 0x30;

const STATUS_FIFO_ERROR: u16 = 1 << 4;
const STATUS_BUFFER_COMPLETE: u16 = 1 << 3;
const STATUS_LAST_VALID: u16 = 1 << 2;
const STATUS_HALTED: u16 = 1;
const STATUS_CLEAR: u16 = STATUS_FIFO_ERROR | STATUS_BUFFER_COMPLETE | STATUS_LAST_VALID;
const CONTROL_RUN: u8 = 1;
const CONTROL_RESET: u8 = 1 << 1;
const EXTENDED_VARIABLE_RATE: u16 = 1;
const BDL_IOC: u32 = 1 << 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u8,
    pub bits: u8,
}

impl PcmFormat {
    pub const STEREO_S16_48K: Self = Self {
        sample_rate: PCM_RATE as u32,
        channels: 2,
        bits: 16,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    InvalidPciBar,
    PciCommand,
    RegisterAccess,
    CodecUnavailable,
    DmaUnavailable,
    AddressTooWide,
    InvalidSamples,
    SilentFallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor: u16,
    pub device: u16,
    pub nam: u16,
    pub nabm: u16,
    pub irq: u8,
    pub codec_ready: bool,
    pub codec_vendor1: u16,
    pub codec_vendor2: u16,
    pub extended_audio_id: u16,
    pub format: PcmFormat,
    pub ring_entries: u8,
    pub buffer_bytes: u16,
    pub running: bool,
    pub recoveries: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitResult {
    Ready(Status),
    Unsupported,
    Failed(AudioError),
}

#[derive(Clone, Copy)]
struct DmaPage {
    physical: u64,
    virtual_address: usize,
}

impl DmaPage {
    fn new() -> Result<Self, AudioError> {
        let physical = crate::memory::alloc_frame().ok_or(AudioError::DmaUnavailable)?;
        let virtual_address =
            crate::arch::physical_to_virtual(crate::address::PhysAddr::new(physical))
                .ok_or(AudioError::DmaUnavailable)?
                .value();
        let page = Self {
            physical,
            virtual_address,
        };
        page.clear();
        page.buffer()
            .validate()
            .map_err(|_| AudioError::DmaUnavailable)?;
        Ok(page)
    }

    fn buffer(self) -> DmaBuffer {
        DmaBuffer {
            physical: crate::address::PhysAddr::new(self.physical),
            virtual_address: crate::address::VirtAddr::new(self.virtual_address),
            length: 4096,
            alignment: 4096,
            direction: DmaDirection::Bidirectional,
            owner: 0,
        }
    }

    fn sync_for_device(self) -> Result<(), AudioError> {
        unsafe { self.buffer().sync_for_device() }.map_err(|_| AudioError::DmaUnavailable)
    }

    fn clear(self) {
        unsafe { core::ptr::write_bytes(self.virtual_address as *mut u8, 0, 4096) };
    }

    fn write_u16(self, offset: usize, value: u16) {
        assert!(offset + 2 <= 4096 && offset.is_multiple_of(2));
        unsafe {
            (self.virtual_address as *mut u16)
                .add(offset / 2)
                .write(value.to_le());
        }
    }

    fn write_u32(self, offset: usize, value: u32) {
        assert!(offset + 4 <= 4096 && offset.is_multiple_of(4));
        unsafe {
            (self.virtual_address as *mut u32)
                .add(offset / 4)
                .write(value.to_le());
        }
    }
}

struct Runtime {
    pci: pci::Device,
    nam: PioRegion,
    nabm: PioRegion,
    bdl: DmaPage,
    data: [DmaPage; RING_ENTRIES],
    irq: u8,
    codec_ready: bool,
    codec_vendor1: u16,
    codec_vendor2: u16,
    extended_audio_id: u16,
    format: PcmFormat,
    running: bool,
    recoveries: u32,
    recovery_logged: bool,
}

static mut RUNTIME: Option<Runtime> = None;

pub fn contract_self_check() {
    assert_eq!(PcmFormat::STEREO_S16_48K.channels, 2);
    assert_eq!(PcmFormat::STEREO_S16_48K.bits, 16);
    assert_eq!(bdl_control(PCM_BYTES), BDL_IOC | (PCM_BYTES as u32 / 2));
    assert!(valid_samples(&[0i16; 2]));
    assert!(!valid_samples(&[0i16; 1]));
    assert!(!valid_samples(&[]));
}

pub fn init() -> InitResult {
    match probe() {
        Ok(Some(runtime)) => {
            let status = runtime.status();
            unsafe { core::ptr::addr_of_mut!(RUNTIME).write(Some(runtime)) };
            InitResult::Ready(status)
        }
        Ok(None) => {
            unsafe { core::ptr::addr_of_mut!(RUNTIME).write(None) };
            InitResult::Unsupported
        }
        Err(error) => {
            unsafe { core::ptr::addr_of_mut!(RUNTIME).write(None) };
            InitResult::Failed(error)
        }
    }
}

pub fn status() -> Option<Status> {
    unsafe {
        (&*core::ptr::addr_of!(RUNTIME))
            .as_ref()
            .map(Runtime::status)
    }
}

pub fn submit_pcm(samples: &[i16]) -> Result<(), AudioError> {
    if !valid_samples(samples) {
        return Err(AudioError::InvalidSamples);
    }
    unsafe {
        let Some(runtime) = (&mut *core::ptr::addr_of_mut!(RUNTIME)).as_mut() else {
            return Err(AudioError::SilentFallback);
        };
        let page = runtime.data[0];
        page.clear();
        for (index, sample) in samples.iter().copied().enumerate() {
            page.write_u16(index * 2, sample as u16);
        }
        page.sync_for_device()?;
        runtime.restart_stream()?;
    }
    Ok(())
}

pub fn poll() {
    unsafe {
        let runtime = core::ptr::addr_of_mut!(RUNTIME);
        if let Some(runtime) = (*runtime).as_mut() {
            runtime.poll();
        }
    }
}

fn probe() -> Result<Option<Runtime>, AudioError> {
    let Some(device) = pci::find_class(AC97_CLASS) else {
        return Ok(None);
    };
    let Some(pci::Bar::Pio(nam_base)) = device.bar(0) else {
        return Err(AudioError::InvalidPciBar);
    };
    let Some(pci::Bar::Pio(nabm_base)) = device.bar(1) else {
        return Err(AudioError::InvalidPciBar);
    };
    if !device.enable(true, false, true) {
        return Err(AudioError::PciCommand);
    }
    let Some(nam) = PioRegion::new(nam_base, NAM_SIZE) else {
        return Err(AudioError::InvalidPciBar);
    };
    let Some(nabm) = PioRegion::new(nabm_base, NABM_SIZE) else {
        return Err(AudioError::InvalidPciBar);
    };
    Runtime::new(device, nam, nabm).map(Some)
}

impl Runtime {
    fn new(device: pci::Device, nam: PioRegion, nabm: PioRegion) -> Result<Self, AudioError> {
        let codec_ready = nabm
            .read_u32(GLOBAL_STATUS)
            .ok_or(AudioError::RegisterAccess)?
            & (1 << 8)
            != 0;
        if !codec_ready {
            return Err(AudioError::CodecUnavailable);
        }
        let codec_vendor1 = nam
            .read_u16(NAM_VENDOR_ID1)
            .ok_or(AudioError::RegisterAccess)?;
        let codec_vendor2 = nam
            .read_u16(NAM_VENDOR_ID2)
            .ok_or(AudioError::RegisterAccess)?;
        let extended_audio_id = nam
            .read_u16(NAM_EXTENDED_AUDIO_ID)
            .ok_or(AudioError::RegisterAccess)?;
        let mut data = [DmaPage {
            physical: 0,
            virtual_address: 0,
        }; RING_ENTRIES];
        for page in &mut data {
            *page = DmaPage::new()?;
        }
        let bdl = DmaPage::new()?;
        if bdl.physical > u32::MAX as u64 || data.iter().any(|page| page.physical > u32::MAX as u64)
        {
            return Err(AudioError::AddressTooWide);
        }
        let format = PcmFormat::STEREO_S16_48K;
        bdl.clear();
        for (index, page) in data.iter().enumerate() {
            bdl.write_u32(index * 8, page.physical as u32);
            bdl.write_u32(index * 8 + 4, bdl_control(PCM_BYTES));
            page.clear();
            page.sync_for_device()?;
        }
        bdl.sync_for_device()?;

        let mut runtime = Self {
            pci: device,
            nam,
            nabm,
            bdl,
            data,
            irq: device.irq_line(),
            codec_ready,
            codec_vendor1,
            codec_vendor2,
            extended_audio_id,
            format,
            running: false,
            recoveries: 0,
            recovery_logged: false,
        };
        runtime.configure_codec()?;
        runtime.restart_stream()?;
        Ok(runtime)
    }

    fn configure_codec(&mut self) -> Result<(), AudioError> {
        let _ = self
            .nam
            .read_u16(NAM_RESET)
            .ok_or(AudioError::RegisterAccess)?;
        self.nam
            .write_u16(NAM_MASTER_VOLUME, 0)
            .then_some(())
            .ok_or(AudioError::RegisterAccess)?;
        self.nam
            .write_u16(NAM_PCM_OUT_VOLUME, 0)
            .then_some(())
            .ok_or(AudioError::RegisterAccess)?;
        let control = self
            .nam
            .read_u16(NAM_EXTENDED_AUDIO_CONTROL)
            .ok_or(AudioError::RegisterAccess)?;
        if !self
            .nam
            .write_u16(NAM_EXTENDED_AUDIO_CONTROL, control | EXTENDED_VARIABLE_RATE)
        {
            return Err(AudioError::RegisterAccess);
        }
        if !self.nam.write_u16(NAM_FRONT_DAC_RATE, PCM_RATE) {
            return Err(AudioError::RegisterAccess);
        }
        Ok(())
    }

    fn restart_stream(&mut self) -> Result<(), AudioError> {
        if !self.nabm.write_u8(PO_CONTROL, CONTROL_RESET) {
            return Err(AudioError::RegisterAccess);
        }
        self.clear_status()?;
        if !self.nabm.write_u32(PO_BDBAR, self.bdl.physical as u32)
            || !self.nabm.write_u8(PO_LVI, (RING_ENTRIES - 1) as u8)
            || !self.nabm.write_u8(PO_CONTROL, CONTROL_RUN)
        {
            return Err(AudioError::RegisterAccess);
        }
        self.running = true;
        Ok(())
    }

    fn clear_status(&self) -> Result<(), AudioError> {
        self.nabm
            .write_u16(PO_STATUS, STATUS_CLEAR)
            .then_some(())
            .ok_or(AudioError::RegisterAccess)
    }

    fn poll(&mut self) {
        let Some(status) = self.nabm.read_u16(PO_STATUS) else {
            if self.running {
                self.running = false;
                crate::bootlog::warn("AC'97 audio status read failed; silent fallback active");
            }
            return;
        };
        if status & STATUS_FIFO_ERROR != 0 {
            self.recoveries = self.recoveries.saturating_add(1);
            if !self.recovery_logged {
                crate::bootlog::warn("AC'97 audio FIFO underrun/overrun; PCM ring recovered");
                self.recovery_logged = true;
            }
            self.recover("FIFO error");
            return;
        }
        if status & (STATUS_HALTED | STATUS_LAST_VALID) != 0 {
            self.recover("stream halted");
            return;
        }
        if status & STATUS_BUFFER_COMPLETE != 0 {
            let _ = self.clear_status();
        }
        if status & (STATUS_BUFFER_COMPLETE | STATUS_LAST_VALID) == 0 {
            self.recovery_logged = false;
        }
    }

    fn recover(&mut self, reason: &'static str) {
        if let Err(error) = self.restart_stream() {
            if self.running {
                crate::bootlog::warn_fmt(format_args!(
                    "AC'97 audio {} recovery failed: {:?}; silent fallback active",
                    reason, error
                ));
            }
            self.running = false;
        }
    }

    fn status(&self) -> Status {
        Status {
            bus: self.pci.address.bus,
            slot: self.pci.address.slot,
            function: self.pci.address.function,
            vendor: self.pci.vendor,
            device: self.pci.device,
            nam: self.nam.base(),
            nabm: self.nabm.base(),
            irq: self.irq,
            codec_ready: self.codec_ready,
            codec_vendor1: self.codec_vendor1,
            codec_vendor2: self.codec_vendor2,
            extended_audio_id: self.extended_audio_id,
            format: self.format,
            ring_entries: RING_ENTRIES as u8,
            buffer_bytes: PCM_BYTES as u16,
            running: self.running,
            recoveries: self.recoveries,
        }
    }
}

fn valid_samples(samples: &[i16]) -> bool {
    !samples.is_empty() && samples.len().is_multiple_of(2) && samples.len() <= PCM_BYTES / 2
}

fn bdl_control(bytes: usize) -> u32 {
    BDL_IOC | (bytes as u32 / 2)
}
