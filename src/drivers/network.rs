use crate::drivers::framework::{DmaBuffer, DmaDirection};
use crate::drivers::pci;
use crate::io::PioRegion;

const VIRTIO_NET_CLASS: u32 = 0x0002_0000;
const PAGE_SIZE: usize = 4096;
const MAX_QUEUE_SIZE: u16 = 256;
const MAX_QUEUE_PAGES: usize = 3;
const RX_BUFFERS: usize = 4;
const NET_HEADER: usize = 10;
const PACKET_BUFFER: usize = 2048;
const MIN_FRAME: usize = 14;
const MAX_FRAME: usize = 1514;

const DEVICE_FEATURES: usize = 0x00;
const GUEST_FEATURES: usize = 0x04;
const QUEUE_ADDRESS: usize = 0x08;
const QUEUE_SIZE: usize = 0x0c;
const QUEUE_SELECT: usize = 0x0e;
const QUEUE_NOTIFY: usize = 0x10;
const DEVICE_STATUS: usize = 0x12;
const ISR_STATUS: usize = 0x13;
const DEVICE_CONFIG: usize = 0x14;

const FEATURE_MAC: u32 = 1 << 5;
const FEATURE_STATUS: u32 = 1 << 16;
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 1 << 1;
const STATUS_DRIVER_OK: u8 = 1 << 2;
const STATUS_FAILED: u8 = 1 << 7;
const LINK_UP: u16 = 1;
const DESC_WRITE: u16 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    InvalidPciBar,
    PciCommand,
    Device,
    UnsupportedFeatures,
    InvalidQueue,
    DmaUnavailable,
    AddressTooWide,
    InvalidPacket,
    Busy,
    WouldBlock,
    BufferTooSmall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
    pub vendor: u16,
    pub device: u16,
    pub mac: [u8; 6],
    pub link_up: bool,
    pub irq: u8,
    pub interrupts: bool,
    pub rx_queue: u16,
    pub tx_queue: u16,
    pub rx_pending: u16,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_dropped: u64,
    pub tx_busy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitResult {
    Ready(Status),
    Unsupported,
    Failed(NetError),
}

#[derive(Clone, Copy)]
struct DmaPage {
    physical: u64,
    virtual_address: usize,
}

impl DmaPage {
    const fn empty() -> Self {
        Self {
            physical: 0,
            virtual_address: 0,
        }
    }

    fn new() -> Result<Self, NetError> {
        let physical = crate::memory::alloc_frame().ok_or(NetError::DmaUnavailable)?;
        let virtual_address =
            crate::arch::physical_to_virtual(crate::address::PhysAddr::new(physical))
                .ok_or(NetError::DmaUnavailable)?
                .value();
        let page = Self {
            physical,
            virtual_address,
        };
        page.clear();
        page.sync_for_device()?;
        Ok(page)
    }

    fn buffer(self, length: usize) -> DmaBuffer {
        DmaBuffer {
            physical: crate::address::PhysAddr::new(self.physical),
            virtual_address: crate::address::VirtAddr::new(self.virtual_address),
            length,
            alignment: PAGE_SIZE,
            direction: DmaDirection::Bidirectional,
            owner: 0,
        }
    }

    fn sync_for_device(self) -> Result<(), NetError> {
        unsafe { self.buffer(PAGE_SIZE).sync_for_device() }.map_err(|_| NetError::DmaUnavailable)
    }

    fn sync_for_cpu(self) -> Result<(), NetError> {
        unsafe { self.buffer(PAGE_SIZE).sync_for_cpu() }.map_err(|_| NetError::DmaUnavailable)
    }

    fn clear(self) {
        unsafe { core::ptr::write_bytes(self.virtual_address as *mut u8, 0, PAGE_SIZE) };
    }

    fn write_bytes(self, offset: usize, bytes: &[u8]) {
        assert!(offset
            .checked_add(bytes.len())
            .is_some_and(|end| end <= PAGE_SIZE));
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                (self.virtual_address + offset) as *mut u8,
                bytes.len(),
            );
        }
    }

    fn copy_to(self, offset: usize, output: &mut [u8]) {
        assert!(offset
            .checked_add(output.len())
            .is_some_and(|end| end <= PAGE_SIZE));
        unsafe {
            core::ptr::copy_nonoverlapping(
                (self.virtual_address + offset) as *const u8,
                output.as_mut_ptr(),
                output.len(),
            );
        }
    }
}

struct QueueMemory {
    pages: [DmaPage; MAX_QUEUE_PAGES],
    page_count: usize,
    queue_size: u16,
    used_offset: usize,
}

impl QueueMemory {
    fn new(queue_size: u16) -> Result<Self, NetError> {
        let Some((_, used_offset, total)) = queue_layout(queue_size) else {
            return Err(NetError::InvalidQueue);
        };
        let page_count = total.div_ceil(PAGE_SIZE);
        if page_count > MAX_QUEUE_PAGES {
            return Err(NetError::InvalidQueue);
        }
        let mut pages = [DmaPage::empty(); MAX_QUEUE_PAGES];
        pages[0] = DmaPage::new()?;
        for index in 1..page_count {
            pages[index] = DmaPage::new()?;
            if pages[index].physical != pages[index - 1].physical.saturating_add(PAGE_SIZE as u64) {
                return Err(NetError::DmaUnavailable);
            }
        }
        let memory = Self {
            pages,
            page_count,
            queue_size,
            used_offset,
        };
        memory.clear();
        memory.sync_for_device()?;
        Ok(memory)
    }

    fn physical(&self) -> u64 {
        self.pages[0].physical
    }

    fn virtual_address(&self) -> usize {
        self.pages[0].virtual_address
    }

    fn length(&self) -> usize {
        self.page_count * PAGE_SIZE
    }

    fn buffer(&self) -> DmaBuffer {
        DmaBuffer {
            physical: crate::address::PhysAddr::new(self.physical()),
            virtual_address: crate::address::VirtAddr::new(self.virtual_address()),
            length: self.length(),
            alignment: PAGE_SIZE,
            direction: DmaDirection::Bidirectional,
            owner: 0,
        }
    }

    fn clear(&self) {
        unsafe { core::ptr::write_bytes(self.virtual_address() as *mut u8, 0, self.length()) };
    }

    fn sync_for_device(&self) -> Result<(), NetError> {
        unsafe { self.buffer().sync_for_device() }.map_err(|_| NetError::DmaUnavailable)
    }

    fn sync_for_cpu(&self) -> Result<(), NetError> {
        unsafe { self.buffer().sync_for_cpu() }.map_err(|_| NetError::DmaUnavailable)
    }

    fn write_u16(&self, offset: usize, value: u16) {
        assert!(offset + 2 <= self.length() && offset.is_multiple_of(2));
        unsafe {
            ((self.virtual_address() + offset) as *mut u16).write_volatile(value.to_le());
        }
    }

    fn write_u32(&self, offset: usize, value: u32) {
        assert!(offset + 4 <= self.length() && offset.is_multiple_of(4));
        unsafe {
            ((self.virtual_address() + offset) as *mut u32).write_volatile(value.to_le());
        }
    }

    fn write_u64(&self, offset: usize, value: u64) {
        assert!(offset + 8 <= self.length() && offset.is_multiple_of(8));
        unsafe {
            ((self.virtual_address() + offset) as *mut u64).write_volatile(value.to_le());
        }
    }

    fn read_u16(&self, offset: usize) -> u16 {
        assert!(offset + 2 <= self.length() && offset.is_multiple_of(2));
        let value = unsafe { ((self.virtual_address() + offset) as *const u16).read_volatile() };
        u16::from_le(value)
    }
}

struct Queue {
    memory: QueueMemory,
    index: u16,
    available: u16,
    used: u16,
}

impl Queue {
    fn new(io: PioRegion, index: u16, size: u16) -> Result<Self, NetError> {
        let memory = QueueMemory::new(size)?;
        if memory.physical() / PAGE_SIZE as u64 > u32::MAX as u64 {
            return Err(NetError::AddressTooWide);
        }
        if !io.write_u16(QUEUE_SELECT, index)
            || !io.write_u32(QUEUE_ADDRESS, (memory.physical() / PAGE_SIZE as u64) as u32)
        {
            return Err(NetError::Device);
        }
        Ok(Self {
            memory,
            index,
            available: 0,
            used: 0,
        })
    }

    fn size(&self) -> u16 {
        self.memory.queue_size
    }

    fn descriptor_offset(&self, index: u16) -> usize {
        index as usize * 16
    }

    fn available_offset(&self) -> usize {
        self.memory.queue_size as usize * 16
    }

    fn set_descriptor(&self, index: u16, physical: u64, length: u32, flags: u16) {
        let offset = self.descriptor_offset(index);
        self.memory.write_u64(offset, physical);
        self.memory.write_u32(offset + 8, length);
        self.memory.write_u16(offset + 12, flags);
        self.memory.write_u16(offset + 14, 0);
    }

    fn publish(&mut self, descriptor: u16) {
        let ring = self.available_offset() + 4 + (self.available % self.size()) as usize * 2;
        self.memory.write_u16(ring, descriptor);
        self.available = self.available.wrapping_add(1);
        self.memory
            .write_u16(self.available_offset() + 2, self.available);
    }

    fn take_used(&mut self) -> Result<Option<(u32, u32)>, NetError> {
        let device_used = self.memory.read_u16(self.memory.used_offset + 2);
        let pending = device_used.wrapping_sub(self.used);
        if pending == 0 {
            return Ok(None);
        }
        if pending > self.size() {
            return Err(NetError::InvalidQueue);
        }
        let offset = self.memory.used_offset + 4 + (self.used % self.size()) as usize * 8;
        let descriptor = self.memory.read_u32(offset);
        let length = self.memory.read_u32(offset + 4);
        self.used = self.used.wrapping_add(1);
        Ok(Some((descriptor, length)))
    }
}

impl QueueMemory {
    fn read_u32(&self, offset: usize) -> u32 {
        assert!(offset + 4 <= self.length() && offset.is_multiple_of(4));
        let value = unsafe { ((self.virtual_address() + offset) as *const u32).read_volatile() };
        u32::from_le(value)
    }
}

struct Runtime {
    pci: pci::Device,
    io: PioRegion,
    rx: Queue,
    tx: Queue,
    rx_buffers: [DmaPage; RX_BUFFERS],
    tx_buffer: DmaPage,
    mac: [u8; 6],
    link_up: bool,
    irq: u8,
    interrupts: bool,
    tx_busy: bool,
    rx_packets: u64,
    tx_packets: u64,
    rx_dropped: u64,
    tx_error_logged: bool,
}

static mut RUNTIME: Option<Runtime> = None;

pub fn contract_self_check() {
    let (available, used, total) = queue_layout(8).expect("valid virtio queue layout");
    assert_eq!(available, 128);
    assert_eq!(used, PAGE_SIZE);
    assert!(total > used);
    assert!(valid_packet(&[0; MIN_FRAME]));
    assert!(!valid_packet(&[]));
    assert!(!valid_packet(&[0; MAX_FRAME + 1]));
    assert!(queue_layout(0).is_none());
    assert!(queue_layout(3).is_none());
    assert!(queue_layout(MAX_QUEUE_SIZE - 1).is_none());
}

pub fn init() -> InitResult {
    match probe() {
        Ok(Some(mut runtime)) => {
            runtime.interrupts = runtime.register_interrupt();
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

pub fn poll() {
    unsafe {
        let runtime = core::ptr::addr_of_mut!(RUNTIME);
        if let Some(runtime) = (*runtime).as_mut() {
            runtime.poll();
        }
    }
}

pub fn transmit_packet(packet: &[u8]) -> Result<(), NetError> {
    unsafe {
        let Some(runtime) = (&mut *core::ptr::addr_of_mut!(RUNTIME)).as_mut() else {
            return Err(NetError::Device);
        };
        runtime.transmit(packet)
    }
}

pub fn receive_packet(output: &mut [u8]) -> Result<usize, NetError> {
    unsafe {
        let Some(runtime) = (&mut *core::ptr::addr_of_mut!(RUNTIME)).as_mut() else {
            return Err(NetError::Device);
        };
        runtime.receive(output)
    }
}

fn probe() -> Result<Option<Runtime>, NetError> {
    let Some(device) = pci::find_class_vendor(VIRTIO_NET_CLASS, Some(0x1af4)) else {
        return Ok(None);
    };
    let Some(pci::Bar::Pio(base)) = device.bar(0) else {
        return Err(NetError::InvalidPciBar);
    };
    if !device.enable(true, false, true) {
        return Err(NetError::PciCommand);
    }
    let Some(io) = PioRegion::new(base, 0x100) else {
        return Err(NetError::InvalidPciBar);
    };
    Runtime::new(device, io).map(Some)
}

impl Runtime {
    fn new(device: pci::Device, io: PioRegion) -> Result<Self, NetError> {
        let features = io.read_u32(DEVICE_FEATURES).ok_or(NetError::Device)?;
        if !io.write_u8(DEVICE_STATUS, 0)
            || !io.write_u8(DEVICE_STATUS, STATUS_ACKNOWLEDGE)
            || !io.write_u8(DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER)
        {
            return Err(NetError::Device);
        }
        let negotiated = features & (FEATURE_MAC | FEATURE_STATUS);
        if negotiated != FEATURE_MAC | FEATURE_STATUS {
            let _ = io.write_u8(DEVICE_STATUS, STATUS_FAILED);
            return Err(NetError::UnsupportedFeatures);
        }
        if !io.write_u32(GUEST_FEATURES, negotiated) {
            return Err(NetError::Device);
        }
        let mut mac = [0; 6];
        for (index, byte) in mac.iter_mut().enumerate() {
            *byte = io.read_u8(DEVICE_CONFIG + index).ok_or(NetError::Device)?;
        }
        let link_up = io.read_u16(DEVICE_CONFIG + 6).ok_or(NetError::Device)? & LINK_UP != 0;
        let rx_size = queue_size(io, 0)?;
        let tx_size = queue_size(io, 1)?;
        if rx_size < RX_BUFFERS as u16 || tx_size == 0 {
            return Err(NetError::InvalidQueue);
        }
        let mut rx_buffers = [DmaPage::empty(); RX_BUFFERS];
        for buffer in &mut rx_buffers {
            *buffer = DmaPage::new()?;
        }
        let tx_buffer = DmaPage::new()?;
        if rx_buffers
            .iter()
            .chain(core::iter::once(&tx_buffer))
            .any(|page| page.physical > (u32::MAX as u64) << 12)
        {
            return Err(NetError::AddressTooWide);
        }
        let mut rx = Queue::new(io, 0, rx_size)?;
        let tx = Queue::new(io, 1, tx_size)?;
        for (index, buffer) in rx_buffers.iter().enumerate() {
            rx.set_descriptor(
                index as u16,
                buffer.physical,
                PACKET_BUFFER as u32,
                DESC_WRITE,
            );
            rx.publish(index as u16);
        }
        rx.memory.sync_for_device()?;
        tx.set_descriptor(0, tx_buffer.physical, PAGE_SIZE as u32, 0);
        tx.memory.sync_for_device()?;
        if !io.write_u8(
            DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        ) {
            return Err(NetError::Device);
        }
        Ok(Self {
            pci: device,
            io,
            rx,
            tx,
            rx_buffers,
            tx_buffer,
            mac,
            link_up,
            irq: device.irq_line(),
            interrupts: false,
            tx_busy: false,
            rx_packets: 0,
            tx_packets: 0,
            rx_dropped: 0,
            tx_error_logged: false,
        })
    }

    fn register_interrupt(&mut self) -> bool {
        if self.irq > 15 {
            return false;
        }
        let vector = 32u32 + self.irq as u32;
        let Ok(_) = crate::irq::register(
            crate::drivers::framework::IrqKind::Legacy,
            self.irq as u32,
            vector,
            interrupt_hard,
            Some(interrupt_deferred),
        ) else {
            return false;
        };
        crate::arch::enable_legacy_irq(self.irq);
        true
    }

    fn poll(&mut self) {
        let Some(link) = self.io.read_u16(DEVICE_CONFIG + 6) else {
            if !self.tx_error_logged {
                crate::bootlog::warn("virtio-net link status read failed; networking deferred");
                self.tx_error_logged = true;
            }
            return;
        };
        let link_up = link & LINK_UP != 0;
        if link_up != self.link_up {
            self.link_up = link_up;
            if link_up {
                crate::bootlog::ok("virtio-net link up");
            } else {
                crate::bootlog::warn("virtio-net link down");
            }
        }
        if self.tx.memory.sync_for_cpu().is_err() {
            if !self.tx_error_logged {
                crate::bootlog::warn("virtio-net TX completion sync failed; networking deferred");
                self.tx_error_logged = true;
            }
            return;
        }
        for _ in 0..self.tx.size() {
            let Ok(Some((descriptor, _))) = self.tx.take_used() else {
                break;
            };
            if descriptor == 0 {
                self.tx_busy = false;
                self.tx_packets = self.tx_packets.saturating_add(1);
            }
        }
    }

    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError> {
        self.poll();
        if !valid_packet(packet) {
            return Err(NetError::InvalidPacket);
        }
        if self.tx_busy {
            return Err(NetError::Busy);
        }
        let frame_len = packet.len().max(60);
        self.tx_buffer.clear();
        self.tx_buffer.write_bytes(NET_HEADER, packet);
        self.tx_buffer.sync_for_device()?;
        self.tx.set_descriptor(
            0,
            self.tx_buffer.physical,
            (NET_HEADER + frame_len) as u32,
            0,
        );
        self.tx.publish(0);
        self.tx.memory.sync_for_device()?;
        if !self.tx.io_notify(self.io) {
            return Err(NetError::Device);
        }
        self.tx_busy = true;
        Ok(())
    }

    fn receive(&mut self, output: &mut [u8]) -> Result<usize, NetError> {
        self.rx.memory.sync_for_cpu()?;
        let Some((descriptor, length)) = self.rx.take_used()? else {
            return Err(NetError::WouldBlock);
        };
        let result = if descriptor as usize >= RX_BUFFERS || (length as usize) < NET_HEADER {
            self.rx_dropped = self.rx_dropped.saturating_add(1);
            Err(NetError::InvalidPacket)
        } else {
            let frame_len = length as usize - NET_HEADER;
            if frame_len > MAX_FRAME || frame_len > output.len() {
                self.rx_dropped = self.rx_dropped.saturating_add(1);
                Err(NetError::BufferTooSmall)
            } else {
                let buffer = self.rx_buffers[descriptor as usize];
                buffer.sync_for_cpu()?;
                buffer.copy_to(NET_HEADER, &mut output[..frame_len]);
                self.rx_packets = self.rx_packets.saturating_add(1);
                Ok(frame_len)
            }
        };
        let descriptor = descriptor as usize;
        if descriptor < RX_BUFFERS {
            self.rx.publish(descriptor as u16);
            self.rx.memory.sync_for_device()?;
        }
        result
    }

    fn status(&self) -> Status {
        let rx_pending = self
            .rx
            .memory
            .read_u16(self.rx.memory.used_offset + 2)
            .wrapping_sub(self.rx.used);
        Status {
            bus: self.pci.address.bus,
            slot: self.pci.address.slot,
            function: self.pci.address.function,
            vendor: self.pci.vendor,
            device: self.pci.device,
            mac: self.mac,
            link_up: self.link_up,
            irq: self.irq,
            interrupts: self.interrupts,
            rx_queue: self.rx.size(),
            tx_queue: self.tx.size(),
            rx_pending,
            rx_packets: self.rx_packets,
            tx_packets: self.tx_packets,
            rx_dropped: self.rx_dropped,
            tx_busy: self.tx_busy,
        }
    }
}

impl Queue {
    fn io_notify(&self, io: PioRegion) -> bool {
        io.write_u16(QUEUE_NOTIFY, self.index)
    }
}

fn queue_size(io: PioRegion, index: u16) -> Result<u16, NetError> {
    if !io.write_u16(QUEUE_SELECT, index) {
        return Err(NetError::Device);
    }
    let size = io.read_u16(QUEUE_SIZE).ok_or(NetError::Device)?;
    if size == 0 || size > MAX_QUEUE_SIZE || !size.is_power_of_two() {
        return Err(NetError::InvalidQueue);
    }
    Ok(size)
}

fn queue_layout(queue_size: u16) -> Option<(usize, usize, usize)> {
    if queue_size == 0 || queue_size > MAX_QUEUE_SIZE || !queue_size.is_power_of_two() {
        return None;
    }
    let available = queue_size as usize * 16;
    let available_end = available.checked_add(4 + queue_size as usize * 2)?;
    let used = available_end.checked_add(PAGE_SIZE - 1)? & !(PAGE_SIZE - 1);
    let total = used.checked_add(4 + queue_size as usize * 8)?;
    Some((available, used, total))
}

fn valid_packet(packet: &[u8]) -> bool {
    (MIN_FRAME..=MAX_FRAME).contains(&packet.len())
}

fn interrupt_hard() -> bool {
    unsafe {
        let runtime = core::ptr::addr_of!(RUNTIME);
        (*runtime)
            .as_ref()
            .and_then(|runtime| runtime.io.read_u8(ISR_STATUS))
            .is_some_and(|status| status != 0)
    }
}

fn interrupt_deferred() {
    poll();
}
