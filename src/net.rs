use crate::drivers::network as device;

const MAX_FRAME: usize = 1514;
const MAX_ARP: usize = 8;
const MAX_UDP_PAYLOAD: usize = 512;
const MAX_TCP_PAYLOAD: usize = 512;
const ARP_ETHERNET: u16 = 1;
const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_ARP: u16 = 0x0806;
const IP_PROTOCOL_ICMP: u8 = 1;
const IP_PROTOCOL_TCP: u8 = 6;
const IP_PROTOCOL_UDP: u8 = 17;
const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_MESSAGE_TYPE: u8 = 53;
const DNS_PORT: u16 = 53;
const UDP_SOCKET_PORT: u16 = 49152;
const DNS_SOCKET_PORT: u16 = 49153;
const TCP_SOCKET_PORT: u16 = 49154;
const DHCP_RETRY_LIMIT: u8 = 3;
const TCP_RETRY_LIMIT: u8 = 3;
const DHCP_RETRY_SCHEDULER_TICKS: u64 = 100;
const DNS_TIMEOUT_SCHEDULER_TICKS: u64 = 500;
const TCP_RETRY_SCHEDULER_TICKS: u64 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const ZERO: Self = Self([0, 0, 0, 0]);
    pub const BROADCAST: Self = Self([255, 255, 255, 255]);

    pub fn parse(input: &str) -> Option<Self> {
        let mut address = [0; 4];
        let mut count = 0;
        for part in input.split('.') {
            if count == 4 || part.is_empty() {
                return None;
            }
            let mut value = 0u16;
            for byte in part.bytes() {
                if !byte.is_ascii_digit() {
                    return None;
                }
                value = value.checked_mul(10)?.checked_add((byte - b'0') as u16)?;
                if value > 255 {
                    return None;
                }
            }
            address[count] = value as u8;
            count += 1;
        }
        (count == 4).then_some(Self(address))
    }

    fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackError {
    Unsupported,
    Device,
    InvalidAddress,
    InvalidPacket,
    NoRoute,
    WouldBlock,
    Busy,
    PayloadTooLarge,
    NotConnected,
    Protocol,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitResult {
    Ready(Status),
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub mac: [u8; 6],
    pub link_up: bool,
    pub configured: bool,
    pub ip: Ipv4Addr,
    pub gateway: Ipv4Addr,
    pub dns: Ipv4Addr,
    pub arp_entries: u8,
    pub rx_frames: u64,
    pub rx_dropped: u64,
    pub tx_frames: u64,
    pub dhcp: DhcpState,
    pub dns_pending: bool,
    pub dns_result: Option<Ipv4Addr>,
    pub tcp: TcpState,
    pub tcp_rx_bytes: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DhcpState {
    Idle,
    Discovering,
    Requesting,
    Bound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    SynSent,
    Established,
}

#[derive(Clone, Copy)]
struct ArpEntry {
    valid: bool,
    address: Ipv4Addr,
    mac: [u8; 6],
}

impl ArpEntry {
    const EMPTY: Self = Self {
        valid: false,
        address: Ipv4Addr::ZERO,
        mac: [0; 6],
    };
}

#[derive(Clone, Copy)]
struct UdpSocket {
    port: u16,
    last_payload: [u8; MAX_UDP_PAYLOAD],
    last_len: usize,
    last_source: Ipv4Addr,
    last_port: u16,
    dropped: u64,
}

impl UdpSocket {
    const fn new(port: u16) -> Self {
        Self {
            port,
            last_payload: [0; MAX_UDP_PAYLOAD],
            last_len: 0,
            last_source: Ipv4Addr::ZERO,
            last_port: 0,
            dropped: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct DnsState {
    pending: bool,
    id: u16,
    result: Option<Ipv4Addr>,
    started: u64,
}

impl DnsState {
    const fn new() -> Self {
        Self {
            pending: false,
            id: 0,
            result: None,
            started: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct TcpSocket {
    state: TcpState,
    remote: Ipv4Addr,
    remote_port: u16,
    sequence: u32,
    acknowledgement: u32,
    pending: [u8; MAX_TCP_PAYLOAD],
    pending_len: usize,
    pending_sequence: u32,
    retries: u8,
    last_action: u64,
    received: [u8; MAX_TCP_PAYLOAD],
    received_len: usize,
}

impl TcpSocket {
    const fn new() -> Self {
        Self {
            state: TcpState::Closed,
            remote: Ipv4Addr::ZERO,
            remote_port: 0,
            sequence: 0,
            acknowledgement: 0,
            pending: [0; MAX_TCP_PAYLOAD],
            pending_len: 0,
            pending_sequence: 0,
            retries: 0,
            last_action: 0,
            received: [0; MAX_TCP_PAYLOAD],
            received_len: 0,
        }
    }
}

struct Stack {
    mac: [u8; 6],
    link_up: bool,
    configured: bool,
    ip: Ipv4Addr,
    subnet: Ipv4Addr,
    gateway: Ipv4Addr,
    dns_server: Ipv4Addr,
    arp: [ArpEntry; MAX_ARP],
    arp_next: usize,
    dhcp: DhcpState,
    dhcp_xid: u32,
    dhcp_offered: Ipv4Addr,
    dhcp_server: Ipv4Addr,
    dhcp_subnet: Ipv4Addr,
    dhcp_gateway: Ipv4Addr,
    dhcp_dns: Ipv4Addr,
    dhcp_retries: u8,
    dhcp_last_action: u64,
    udp: UdpSocket,
    dns: DnsState,
    tcp: TcpSocket,
    rx_frames: u64,
    rx_dropped: u64,
    tx_frames: u64,
    error_logged: bool,
    next_identifier: u16,
}

static mut STACK: Option<Stack> = None;

pub fn contract_self_check() {
    assert_eq!(Ipv4Addr::parse("10.0.2.15"), Some(Ipv4Addr([10, 0, 2, 15])));
    assert!(Ipv4Addr::parse("10.0.2.256").is_none());
    assert!(Ipv4Addr::parse("10.0.2").is_none());
    let mut header = [0u8; 20];
    write_u16(&mut header, 0, 0x4500);
    write_u16(&mut header, 2, 20);
    write_u16(&mut header, 10, 0);
    let value = checksum(&header);
    write_u16(&mut header, 10, value);
    assert_eq!(checksum(&header), 0);
    assert!(valid_dns_name("example.com"));
    assert!(!valid_dns_name(""));
    let mut frame = [0u8; MAX_FRAME];
    assert!(build_udp_frame(
        &mut frame,
        [0xff; 6],
        [0x52, 0x54, 0, 0x12, 0x34, 0x56],
        Ipv4Addr([10, 0, 2, 15]),
        Ipv4Addr([10, 0, 2, 2]),
        UDP_SOCKET_PORT,
        DNS_PORT,
        b"dns",
        1,
    )
    .is_ok());
    assert_eq!(frame[22], 64);
    assert_eq!(frame[23], IP_PROTOCOL_UDP);
    assert_eq!(&frame[26..30], &[10, 0, 2, 15]);
    assert_eq!(&frame[30..34], &[10, 0, 2, 2]);
    assert_eq!(read_u16(&frame, 34), Some(UDP_SOCKET_PORT));
    assert_eq!(checksum(&frame[14..34]), 0);
}

pub fn init() -> InitResult {
    let Some(device_status) = device::status() else {
        unsafe { core::ptr::addr_of_mut!(STACK).write(None) };
        return InitResult::Unsupported;
    };
    let mut stack = Stack::new(device_status.mac, device_status.link_up);
    stack.dhcp = DhcpState::Discovering;
    stack.dhcp_last_action = crate::time::ticks();
    if let Err(error) = stack.send_dhcp_discover() {
        crate::bootlog::warn_fmt(format_args!(
            "DHCP discover was not queued: {:?}; network stack remains available",
            error
        ));
    }
    let status = stack.status();
    unsafe { core::ptr::addr_of_mut!(STACK).write(Some(stack)) };
    InitResult::Ready(status)
}

pub fn status() -> Option<Status> {
    unsafe { (&*core::ptr::addr_of!(STACK)).as_ref().map(Stack::status) }
}

pub fn poll() {
    unsafe {
        let stack = core::ptr::addr_of_mut!(STACK);
        if let Some(stack) = (*stack).as_mut() {
            stack.poll();
        }
    }
}

pub fn ping(address: Ipv4Addr) -> Result<(), StackError> {
    unsafe {
        let Some(stack) = (&mut *core::ptr::addr_of_mut!(STACK)).as_mut() else {
            return Err(StackError::Unsupported);
        };
        stack.send_ping(address)
    }
}

pub fn udp_send(address: Ipv4Addr, port: u16, payload: &[u8]) -> Result<(), StackError> {
    unsafe {
        let Some(stack) = (&mut *core::ptr::addr_of_mut!(STACK)).as_mut() else {
            return Err(StackError::Unsupported);
        };
        stack.send_udp(address, port, payload)
    }
}

pub fn udp_receive(output: &mut [u8]) -> Result<(Ipv4Addr, u16, usize), StackError> {
    unsafe {
        let Some(stack) = (&mut *core::ptr::addr_of_mut!(STACK)).as_mut() else {
            return Err(StackError::Unsupported);
        };
        if stack.udp.last_len == 0 {
            return Err(StackError::WouldBlock);
        }
        if output.len() < stack.udp.last_len {
            return Err(StackError::PayloadTooLarge);
        }
        let length = stack.udp.last_len;
        output[..length].copy_from_slice(&stack.udp.last_payload[..length]);
        stack.udp.last_len = 0;
        Ok((stack.udp.last_source, stack.udp.last_port, length))
    }
}

pub fn dns_query(name: &str) -> Result<(), StackError> {
    unsafe {
        let Some(stack) = (&mut *core::ptr::addr_of_mut!(STACK)).as_mut() else {
            return Err(StackError::Unsupported);
        };
        stack.start_dns_query(name)
    }
}

pub fn tcp_connect(address: Ipv4Addr, port: u16) -> Result<(), StackError> {
    unsafe {
        let Some(stack) = (&mut *core::ptr::addr_of_mut!(STACK)).as_mut() else {
            return Err(StackError::Unsupported);
        };
        stack.start_tcp(address, port)
    }
}

pub fn tcp_send(payload: &[u8]) -> Result<(), StackError> {
    unsafe {
        let Some(stack) = (&mut *core::ptr::addr_of_mut!(STACK)).as_mut() else {
            return Err(StackError::Unsupported);
        };
        stack.send_tcp(payload)
    }
}

pub fn tcp_receive(output: &mut [u8]) -> Result<usize, StackError> {
    unsafe {
        let Some(stack) = (&mut *core::ptr::addr_of_mut!(STACK)).as_mut() else {
            return Err(StackError::Unsupported);
        };
        if stack.tcp.received_len == 0 {
            return Err(StackError::WouldBlock);
        }
        if output.len() < stack.tcp.received_len {
            return Err(StackError::PayloadTooLarge);
        }
        let length = stack.tcp.received_len;
        output[..length].copy_from_slice(&stack.tcp.received[..length]);
        stack.tcp.received_len = 0;
        Ok(length)
    }
}

#[derive(Clone, Copy)]
struct EthernetPacket {
    source: [u8; 6],
    destination: [u8; 6],
    ethertype: u16,
}

#[derive(Clone, Copy)]
struct Ipv4Packet {
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: u8,
    payload: usize,
    length: usize,
}

#[derive(Clone, Copy)]
struct UdpPacket {
    source_port: u16,
    destination_port: u16,
    payload: usize,
    length: usize,
}

#[derive(Clone, Copy)]
struct TcpPacket {
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    acknowledgement: u32,
    flags: u16,
    payload: usize,
    length: usize,
}

impl Stack {
    fn new(mac: [u8; 6], link_up: bool) -> Self {
        Self {
            mac,
            link_up,
            configured: false,
            ip: Ipv4Addr::ZERO,
            subnet: Ipv4Addr::ZERO,
            gateway: Ipv4Addr::ZERO,
            dns_server: Ipv4Addr::ZERO,
            arp: [ArpEntry::EMPTY; MAX_ARP],
            arp_next: 0,
            dhcp: DhcpState::Idle,
            dhcp_xid: crate::time::ticks() as u32 ^ u32::from(mac[4]) << 8 ^ u32::from(mac[5]),
            dhcp_offered: Ipv4Addr::ZERO,
            dhcp_server: Ipv4Addr::ZERO,
            dhcp_subnet: Ipv4Addr::ZERO,
            dhcp_gateway: Ipv4Addr::ZERO,
            dhcp_dns: Ipv4Addr::ZERO,
            dhcp_retries: 0,
            dhcp_last_action: 0,
            udp: UdpSocket::new(UDP_SOCKET_PORT),
            dns: DnsState::new(),
            tcp: TcpSocket::new(),
            rx_frames: 0,
            rx_dropped: 0,
            tx_frames: 0,
            error_logged: false,
            next_identifier: 1,
        }
    }

    fn poll(&mut self) {
        for _ in 0..4 {
            let mut frame = [0u8; MAX_FRAME];
            match device::receive_packet(&mut frame) {
                Ok(length) => {
                    self.rx_frames = self.rx_frames.saturating_add(1);
                    if self.handle_frame(&frame[..length]).is_err() {
                        self.rx_dropped = self.rx_dropped.saturating_add(1);
                    }
                }
                Err(device::NetError::WouldBlock) => break,
                Err(_) => {
                    self.rx_dropped = self.rx_dropped.saturating_add(1);
                    if !self.error_logged {
                        crate::bootlog::warn(
                            "network RX path failed; malformed frames are being dropped",
                        );
                        self.error_logged = true;
                    }
                    break;
                }
            }
        }
        self.poll_dhcp();
        self.poll_dns();
        self.poll_tcp();
    }

    fn status(&self) -> Status {
        let arp_entries = self.arp.iter().filter(|entry| entry.valid).count() as u8;
        Status {
            mac: self.mac,
            link_up: self.link_up,
            configured: self.configured,
            ip: self.ip,
            gateway: self.gateway,
            dns: self.dns_server,
            arp_entries,
            rx_frames: self.rx_frames,
            rx_dropped: self.rx_dropped,
            tx_frames: self.tx_frames,
            dhcp: self.dhcp,
            dns_pending: self.dns.pending,
            dns_result: self.dns.result,
            tcp: self.tcp.state,
            tcp_rx_bytes: self.tcp.received_len as u16,
        }
    }

    fn handle_frame(&mut self, frame: &[u8]) -> Result<(), StackError> {
        let ethernet = parse_ethernet(frame).ok_or(StackError::InvalidPacket)?;
        if ethernet.destination != self.mac && ethernet.destination != [0xff; 6] {
            return Ok(());
        }
        match ethernet.ethertype {
            ETHERTYPE_ARP => self.handle_arp(frame),
            ETHERTYPE_IPV4 => {
                let packet = parse_ipv4(frame).ok_or(StackError::InvalidPacket)?;
                if !self.accepts_ip(packet.destination)
                    && !self.accepts_dhcp_response(frame, packet)
                {
                    return Ok(());
                }
                if self.configured && !packet.source.is_zero() {
                    self.learn_arp(packet.source, ethernet.source);
                }
                match packet.protocol {
                    IP_PROTOCOL_ICMP => self.handle_icmp(frame, ethernet, packet),
                    IP_PROTOCOL_UDP => self.handle_udp(frame, packet),
                    IP_PROTOCOL_TCP => self.handle_tcp(frame, packet),
                    _ => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }

    fn accepts_ip(&self, destination: Ipv4Addr) -> bool {
        destination == Ipv4Addr::BROADCAST || destination == self.ip
    }

    fn accepts_dhcp_response(&self, frame: &[u8], packet: Ipv4Packet) -> bool {
        if self.configured || packet.protocol != IP_PROTOCOL_UDP {
            return false;
        }
        parse_udp(frame, packet).is_ok_and(|udp| {
            udp.source_port == DHCP_SERVER_PORT && udp.destination_port == DHCP_CLIENT_PORT
        })
    }

    fn handle_arp(&mut self, frame: &[u8]) -> Result<(), StackError> {
        if frame.len() < 42
            || read_u16(frame, 14) != Some(ARP_ETHERNET)
            || read_u16(frame, 16) != Some(ETHERTYPE_IPV4)
            || frame[18] != 6
            || frame[19] != 4
        {
            return Err(StackError::InvalidPacket);
        }
        let operation = read_u16(frame, 20).ok_or(StackError::InvalidPacket)?;
        let mut sender_mac = [0; 6];
        sender_mac.copy_from_slice(&frame[22..28]);
        let sender_ip = Ipv4Addr([frame[28], frame[29], frame[30], frame[31]]);
        let mut target_mac = [0; 6];
        target_mac.copy_from_slice(&frame[32..38]);
        let target_ip = Ipv4Addr([frame[38], frame[39], frame[40], frame[41]]);
        self.learn_arp(sender_ip, sender_mac);
        if operation == 1 && target_ip == self.ip && self.configured {
            self.send_arp(2, sender_ip, sender_mac, sender_mac)
        } else if operation == 2 {
            Ok(())
        } else {
            let _ = target_mac;
            Ok(())
        }
    }

    fn handle_icmp(
        &mut self,
        frame: &[u8],
        ethernet: EthernetPacket,
        packet: Ipv4Packet,
    ) -> Result<(), StackError> {
        if packet.length < 8
            || checksum(&frame[packet.payload..packet.payload + packet.length]) != 0
        {
            return Err(StackError::InvalidPacket);
        }
        let kind = frame[packet.payload];
        if kind == 8 && frame[packet.payload + 1] == 0 && self.configured {
            let mut reply = [0u8; MAX_FRAME];
            let length = build_icmp_reply(
                &mut reply,
                self.mac,
                ethernet.source,
                self.ip,
                packet.source,
                &frame[packet.payload..packet.payload + packet.length],
            )?;
            self.transmit(&reply[..length])
        } else if kind == 0 && packet.length >= 8 {
            crate::bootlog::ok("ICMP echo reply received");
            Ok(())
        } else {
            Ok(())
        }
    }

    fn handle_udp(&mut self, frame: &[u8], packet: Ipv4Packet) -> Result<(), StackError> {
        let udp = parse_udp(frame, packet)?;
        if udp.destination_port == DHCP_CLIENT_PORT && udp.source_port == DHCP_SERVER_PORT {
            return self.handle_dhcp(&frame[udp.payload..udp.payload + udp.length]);
        }
        if udp.destination_port == DNS_SOCKET_PORT && self.dns.pending {
            return self.handle_dns(&frame[udp.payload..udp.payload + udp.length]);
        }
        if udp.destination_port == self.udp.port {
            if udp.length > MAX_UDP_PAYLOAD {
                self.udp.dropped = self.udp.dropped.saturating_add(1);
            } else {
                self.udp.last_payload[..udp.length]
                    .copy_from_slice(&frame[udp.payload..udp.payload + udp.length]);
                self.udp.last_len = udp.length;
                self.udp.last_source = packet.source;
                self.udp.last_port = udp.source_port;
            }
        }
        Ok(())
    }

    fn handle_tcp(&mut self, frame: &[u8], packet: Ipv4Packet) -> Result<(), StackError> {
        let tcp = parse_tcp(frame, packet)?;
        if tcp.destination_port != TCP_SOCKET_PORT
            || tcp.source_port != self.tcp.remote_port
            || packet.source != self.tcp.remote
        {
            return Ok(());
        }
        if tcp.flags & 0x004 != 0 {
            self.tcp.state = TcpState::Closed;
            self.tcp.pending_len = 0;
            crate::bootlog::warn("TCP peer reset the diagnostic socket");
            return Ok(());
        }
        if self.tcp.state == TcpState::SynSent && tcp.flags & 0x012 == 0x012 {
            self.tcp.acknowledgement = tcp.sequence.wrapping_add(1);
            self.tcp.sequence = self.tcp.sequence.wrapping_add(1);
            self.send_tcp_segment(0x010, &[])?;
            self.tcp.state = TcpState::Established;
            crate::bootlog::ok("TCP diagnostic socket established");
            return Ok(());
        }
        if self.tcp.state != TcpState::Established {
            return Ok(());
        }
        if tcp.flags & 0x010 != 0
            && self.tcp.pending_len != 0
            && tcp.acknowledgement
                >= self
                    .tcp
                    .pending_sequence
                    .wrapping_add(self.tcp.pending_len as u32)
        {
            self.tcp.sequence = self
                .tcp
                .pending_sequence
                .wrapping_add(self.tcp.pending_len as u32);
            self.tcp.pending_len = 0;
        }
        if tcp.length != 0 {
            self.tcp.acknowledgement = tcp.sequence.wrapping_add(tcp.length as u32);
            if tcp.length <= MAX_TCP_PAYLOAD {
                self.tcp.received[..tcp.length]
                    .copy_from_slice(&frame[tcp.payload..tcp.payload + tcp.length]);
                self.tcp.received_len = tcp.length;
            }
            self.send_tcp_segment(0x010, &[])?;
        }
        if tcp.flags & 0x001 != 0 {
            self.tcp.acknowledgement = tcp.sequence.wrapping_add(tcp.length as u32 + 1);
            self.send_tcp_segment(0x010, &[])?;
            self.tcp.state = TcpState::Closed;
        }
        Ok(())
    }

    fn transmit(&mut self, frame: &[u8]) -> Result<(), StackError> {
        device::transmit_packet(frame).map_err(map_device_error)?;
        self.tx_frames = self.tx_frames.saturating_add(1);
        Ok(())
    }
}

fn build_dhcp_payload(
    output: &mut [u8],
    xid: u32,
    mac: [u8; 6],
    message_type: u8,
    requested: Option<Ipv4Addr>,
    server: Option<Ipv4Addr>,
) -> usize {
    output.fill(0);
    output[0] = 1;
    output[1] = 1;
    output[2] = 6;
    write_u32(output, 4, xid);
    write_u16(output, 10, 0x8000);
    output[28..34].copy_from_slice(&mac);
    output[236..240].copy_from_slice(&[99, 130, 83, 99]);
    let mut cursor = 240;
    output[cursor..cursor + 3].copy_from_slice(&[DHCP_MESSAGE_TYPE, 1, message_type]);
    cursor += 3;
    if let Some(address) = requested {
        output[cursor..cursor + 2].copy_from_slice(&[50, 4]);
        output[cursor + 2..cursor + 6].copy_from_slice(&address.0);
        cursor += 6;
    }
    if let Some(address) = server {
        output[cursor..cursor + 2].copy_from_slice(&[54, 4]);
        output[cursor + 2..cursor + 6].copy_from_slice(&address.0);
        cursor += 6;
    }
    output[cursor..cursor + 5].copy_from_slice(&[55, 3, 1, 3, 6]);
    cursor += 5;
    output[cursor] = 255;
    cursor + 1
}

fn build_ethernet(
    frame: &mut [u8],
    destination: [u8; 6],
    source: [u8; 6],
    ethertype: u16,
) -> Result<(), StackError> {
    if frame.len() < 14 {
        return Err(StackError::InvalidPacket);
    }
    frame[..6].copy_from_slice(&destination);
    frame[6..12].copy_from_slice(&source);
    write_u16(frame, 12, ethertype);
    Ok(())
}

fn build_ipv4_header(
    frame: &mut [u8],
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: u8,
    payload_length: usize,
    identifier: u16,
) -> Result<(), StackError> {
    let total = 20usize
        .checked_add(payload_length)
        .ok_or(StackError::PayloadTooLarge)?;
    if total > u16::MAX as usize || frame.len() < 34 + payload_length {
        return Err(StackError::PayloadTooLarge);
    }
    frame[14] = 0x45;
    frame[15] = 0;
    write_u16(frame, 16, total as u16);
    write_u16(frame, 18, identifier);
    write_u16(frame, 20, 0x4000);
    frame[22] = 64;
    frame[23] = protocol;
    write_u16(frame, 24, 0);
    frame[26..30].copy_from_slice(&source.0);
    frame[30..34].copy_from_slice(&destination.0);
    let value = checksum(&frame[14..34]);
    write_u16(frame, 24, value);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_ipv4_frame(
    frame: &mut [u8],
    destination_mac: [u8; 6],
    source_mac: [u8; 6],
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: u8,
    payload: &[u8],
    identifier: u16,
) -> Result<usize, StackError> {
    let total = 14usize
        .checked_add(20)
        .and_then(|value| value.checked_add(payload.len()))
        .ok_or(StackError::PayloadTooLarge)?;
    if total > MAX_FRAME {
        return Err(StackError::PayloadTooLarge);
    }
    build_ethernet(frame, destination_mac, source_mac, ETHERTYPE_IPV4)?;
    build_ipv4_header(
        frame,
        source,
        destination,
        protocol,
        payload.len(),
        identifier,
    )?;
    frame[34..total].copy_from_slice(payload);
    Ok(total)
}

#[allow(clippy::too_many_arguments)]
fn build_udp_frame(
    frame: &mut [u8],
    destination_mac: [u8; 6],
    source_mac: [u8; 6],
    source: Ipv4Addr,
    destination: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    payload: &[u8],
    identifier: u16,
) -> Result<usize, StackError> {
    let total = 14usize
        .checked_add(20)
        .and_then(|value| value.checked_add(8))
        .and_then(|value| value.checked_add(payload.len()))
        .ok_or(StackError::PayloadTooLarge)?;
    if total > MAX_FRAME {
        return Err(StackError::PayloadTooLarge);
    }
    build_ethernet(frame, destination_mac, source_mac, ETHERTYPE_IPV4)?;
    build_ipv4_header(
        frame,
        source,
        destination,
        IP_PROTOCOL_UDP,
        8 + payload.len(),
        identifier,
    )?;
    let udp = 34;
    write_u16(frame, udp, source_port);
    write_u16(frame, udp + 2, destination_port);
    write_u16(frame, udp + 4, (8 + payload.len()) as u16);
    write_u16(frame, udp + 6, 0);
    frame[udp + 8..total].copy_from_slice(payload);
    let value = transport_checksum(source, destination, IP_PROTOCOL_UDP, &frame[udp..total]);
    write_u16(frame, udp + 6, if value == 0 { 0xffff } else { value });
    Ok(total)
}

fn build_icmp_reply(
    frame: &mut [u8],
    source_mac: [u8; 6],
    destination_mac: [u8; 6],
    source: Ipv4Addr,
    destination: Ipv4Addr,
    request: &[u8],
) -> Result<usize, StackError> {
    if request.len() < 8 {
        return Err(StackError::InvalidPacket);
    }
    let mut reply = [0u8; MAX_FRAME - 34];
    if request.len() > reply.len() {
        return Err(StackError::PayloadTooLarge);
    }
    reply[..request.len()].copy_from_slice(request);
    reply[0] = 0;
    write_u16(&mut reply, 2, 0);
    let value = checksum(&reply[..request.len()]);
    write_u16(&mut reply, 2, value);
    build_ipv4_frame(
        frame,
        destination_mac,
        source_mac,
        source,
        destination,
        IP_PROTOCOL_ICMP,
        &reply[..request.len()],
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_tcp_frame(
    frame: &mut [u8],
    destination_mac: [u8; 6],
    source_mac: [u8; 6],
    source: Ipv4Addr,
    destination: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    sequence: u32,
    acknowledgement: u32,
    flags: u16,
    payload: &[u8],
) -> Result<usize, StackError> {
    let total = 14usize
        .checked_add(20)
        .and_then(|value| value.checked_add(20))
        .and_then(|value| value.checked_add(payload.len()))
        .ok_or(StackError::PayloadTooLarge)?;
    if total > MAX_FRAME {
        return Err(StackError::PayloadTooLarge);
    }
    build_ethernet(frame, destination_mac, source_mac, ETHERTYPE_IPV4)?;
    build_ipv4_header(
        frame,
        source,
        destination,
        IP_PROTOCOL_TCP,
        20 + payload.len(),
        0,
    )?;
    let tcp = 34;
    write_u16(frame, tcp, source_port);
    write_u16(frame, tcp + 2, destination_port);
    write_u32(frame, tcp + 4, sequence);
    write_u32(frame, tcp + 8, acknowledgement);
    frame[tcp + 12] = 5 << 4;
    frame[tcp + 13] = flags as u8;
    write_u16(frame, tcp + 14, 4096);
    write_u16(frame, tcp + 16, 0);
    write_u16(frame, tcp + 18, 0);
    frame[tcp + 20..total].copy_from_slice(payload);
    write_u16(
        frame,
        tcp + 16,
        transport_checksum(source, destination, IP_PROTOCOL_TCP, &frame[tcp..total]),
    );
    Ok(total)
}

fn parse_ethernet(frame: &[u8]) -> Option<EthernetPacket> {
    if frame.len() < 14 {
        return None;
    }
    let mut destination = [0; 6];
    let mut source = [0; 6];
    destination.copy_from_slice(&frame[..6]);
    source.copy_from_slice(&frame[6..12]);
    Some(EthernetPacket {
        source,
        destination,
        ethertype: read_u16(frame, 12)?,
    })
}

fn parse_ipv4(frame: &[u8]) -> Option<Ipv4Packet> {
    if frame.len() < 34 || read_u16(frame, 12)? != ETHERTYPE_IPV4 {
        return None;
    }
    let version = frame[14] >> 4;
    let header_length = usize::from(frame[14] & 0x0f) * 4;
    if version != 4 || header_length < 20 || 14 + header_length > frame.len() {
        return None;
    }
    let total = usize::from(read_u16(frame, 16)?);
    if total < header_length
        || 14 + total > frame.len()
        || checksum(&frame[14..14 + header_length]) != 0
    {
        return None;
    }
    let source = Ipv4Addr(frame[26..30].try_into().ok()?);
    let destination = Ipv4Addr(frame[30..34].try_into().ok()?);
    Some(Ipv4Packet {
        source,
        destination,
        protocol: frame[23],
        payload: 14 + header_length,
        length: total - header_length,
    })
}

fn parse_udp(frame: &[u8], packet: Ipv4Packet) -> Result<UdpPacket, StackError> {
    if packet.length < 8 || packet.payload + packet.length > frame.len() {
        return Err(StackError::InvalidPacket);
    }
    let length = usize::from(read_u16(frame, packet.payload + 4).ok_or(StackError::InvalidPacket)?);
    if length < 8 || length > packet.length {
        return Err(StackError::InvalidPacket);
    }
    let checksum_value = read_u16(frame, packet.payload + 6).ok_or(StackError::InvalidPacket)?;
    if checksum_value != 0
        && transport_checksum(
            packet.source,
            packet.destination,
            IP_PROTOCOL_UDP,
            &frame[packet.payload..packet.payload + length],
        ) != 0
    {
        return Err(StackError::InvalidPacket);
    }
    Ok(UdpPacket {
        source_port: read_u16(frame, packet.payload).ok_or(StackError::InvalidPacket)?,
        destination_port: read_u16(frame, packet.payload + 2).ok_or(StackError::InvalidPacket)?,
        payload: packet.payload + 8,
        length: length - 8,
    })
}

fn parse_tcp(frame: &[u8], packet: Ipv4Packet) -> Result<TcpPacket, StackError> {
    if packet.length < 20 || packet.payload + packet.length > frame.len() {
        return Err(StackError::InvalidPacket);
    }
    let header_length = usize::from(frame[packet.payload + 12] >> 4) * 4;
    if header_length < 20 || header_length > packet.length {
        return Err(StackError::InvalidPacket);
    }
    if transport_checksum(
        packet.source,
        packet.destination,
        IP_PROTOCOL_TCP,
        &frame[packet.payload..packet.payload + packet.length],
    ) != 0
    {
        return Err(StackError::InvalidPacket);
    }
    Ok(TcpPacket {
        source_port: read_u16(frame, packet.payload).ok_or(StackError::InvalidPacket)?,
        destination_port: read_u16(frame, packet.payload + 2).ok_or(StackError::InvalidPacket)?,
        sequence: read_u32(frame, packet.payload + 4).ok_or(StackError::InvalidPacket)?,
        acknowledgement: read_u32(frame, packet.payload + 8).ok_or(StackError::InvalidPacket)?,
        flags: u16::from(frame[packet.payload + 13])
            | (u16::from(frame[packet.payload + 12] & 1) << 8),
        payload: packet.payload + header_length,
        length: packet.length - header_length,
    })
}

fn read_u16(buffer: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        buffer.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_u32(buffer: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        buffer.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn write_u16(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn checksum(buffer: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in buffer.chunks(2) {
        sum += u32::from(u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]));
        sum = (sum & 0xffff) + (sum >> 16);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn transport_checksum(
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: u8,
    payload: &[u8],
) -> u16 {
    let mut sum = 0u32;
    for address in [source, destination] {
        sum += u32::from(u16::from_be_bytes([address.0[0], address.0[1]]));
        sum += u32::from(u16::from_be_bytes([address.0[2], address.0[3]]));
    }
    sum += u32::from(protocol);
    sum += payload.len() as u32;
    for chunk in payload.chunks(2) {
        sum += u32::from(u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]));
        sum = (sum & 0xffff) + (sum >> 16);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn same_subnet(left: Ipv4Addr, right: Ipv4Addr, mask: Ipv4Addr) -> bool {
    !mask.is_zero() && ipv4_bits(left) & ipv4_bits(mask) == ipv4_bits(right) & ipv4_bits(mask)
}

fn ipv4_bits(address: Ipv4Addr) -> u32 {
    u32::from_be_bytes(address.0)
}

fn ipv4_from(buffer: &[u8], offset: usize) -> Option<Ipv4Addr> {
    Some(Ipv4Addr(buffer.get(offset..offset + 4)?.try_into().ok()?))
}

fn valid_dns_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 63 || name.starts_with('.') || name.ends_with('.') {
        return false;
    }
    let mut label_length = 0;
    for byte in name.bytes() {
        if byte == b'.' {
            if label_length == 0 {
                return false;
            }
            label_length = 0;
            continue;
        }
        if !(byte.is_ascii_alphanumeric() || byte == b'-') {
            return false;
        }
        label_length += 1;
        if label_length > 63 {
            return false;
        }
    }
    label_length != 0
}

fn encode_dns_name(name: &str, output: &mut [u8]) -> Option<usize> {
    if !valid_dns_name(name) {
        return None;
    }
    let mut cursor = 1;
    let mut label_start = 0;
    let mut label_length = 0;
    for byte in name.bytes() {
        if byte == b'.' {
            *output.get_mut(label_start)? = label_length;
            label_start = cursor;
            cursor += 1;
            label_length = 0;
        } else {
            *output.get_mut(cursor)? = byte;
            cursor += 1;
            label_length += 1;
        }
    }
    *output.get_mut(label_start)? = label_length;
    *output.get_mut(cursor)? = 0;
    Some(cursor + 1)
}

fn skip_dns_name(buffer: &[u8], offset: usize) -> Option<usize> {
    let mut cursor = offset;
    for _ in 0..16 {
        let length = *buffer.get(cursor)?;
        if length == 0 {
            return Some(cursor + 1);
        }
        if length & 0xc0 == 0xc0 {
            return (cursor + 2 <= buffer.len()).then_some(cursor + 2);
        }
        if length > 63 {
            return None;
        }
        cursor = cursor.checked_add(1 + usize::from(length))?;
        if cursor > buffer.len() {
            return None;
        }
    }
    None
}

impl Stack {
    fn learn_arp(&mut self, address: Ipv4Addr, mac: [u8; 6]) {
        if address.is_zero() || address == Ipv4Addr::BROADCAST {
            return;
        }
        if let Some(entry) = self
            .arp
            .iter_mut()
            .find(|entry| entry.valid && entry.address == address)
        {
            entry.mac = mac;
            return;
        }
        let index = self
            .arp
            .iter()
            .position(|entry| !entry.valid)
            .unwrap_or_else(|| {
                let index = self.arp_next % MAX_ARP;
                self.arp_next = self.arp_next.wrapping_add(1);
                index
            });
        self.arp[index] = ArpEntry {
            valid: true,
            address,
            mac,
        };
    }

    fn lookup_arp(&self, address: Ipv4Addr) -> Option<[u8; 6]> {
        self.arp
            .iter()
            .find(|entry| entry.valid && entry.address == address)
            .map(|entry| entry.mac)
    }

    fn next_hop(&mut self, destination: Ipv4Addr) -> Result<([u8; 6], Ipv4Addr), StackError> {
        if !self.configured || destination.is_zero() {
            return Err(StackError::NoRoute);
        }
        let target = if same_subnet(destination, self.ip, self.subnet) {
            destination
        } else if !self.gateway.is_zero() {
            self.gateway
        } else {
            return Err(StackError::NoRoute);
        };
        if let Some(mac) = self.lookup_arp(target) {
            return Ok((mac, target));
        }
        self.send_arp_request(target)?;
        Err(StackError::WouldBlock)
    }

    fn send_arp_request(&mut self, target: Ipv4Addr) -> Result<(), StackError> {
        self.send_arp(1, target, [0; 6], [0xff; 6])
    }

    fn send_arp(
        &mut self,
        operation: u16,
        target_ip: Ipv4Addr,
        target_mac: [u8; 6],
        ethernet_destination: [u8; 6],
    ) -> Result<(), StackError> {
        let mut frame = [0u8; MAX_FRAME];
        build_ethernet(&mut frame, ethernet_destination, self.mac, ETHERTYPE_ARP)?;
        write_u16(&mut frame, 14, ARP_ETHERNET);
        write_u16(&mut frame, 16, ETHERTYPE_IPV4);
        frame[18] = 6;
        frame[19] = 4;
        write_u16(&mut frame, 20, operation);
        frame[22..28].copy_from_slice(&self.mac);
        frame[28..32].copy_from_slice(&self.ip.0);
        frame[32..38].copy_from_slice(&target_mac);
        frame[38..42].copy_from_slice(&target_ip.0);
        self.transmit(&frame[..42])
    }

    fn send_dhcp_discover(&mut self) -> Result<(), StackError> {
        let mut payload = [0u8; 300];
        let length = build_dhcp_payload(&mut payload, self.dhcp_xid, self.mac, 1, None, None);
        let mut frame = [0u8; MAX_FRAME];
        let length = build_udp_frame(
            &mut frame,
            [0xff; 6],
            self.mac,
            Ipv4Addr::ZERO,
            Ipv4Addr::BROADCAST,
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            &payload[..length],
            self.next_identifier,
        )?;
        self.next_identifier = self.next_identifier.wrapping_add(1);
        self.dhcp_last_action = crate::time::ticks();
        self.transmit(&frame[..length])
    }

    fn send_dhcp_request(&mut self) -> Result<(), StackError> {
        let mut payload = [0u8; 300];
        let length = build_dhcp_payload(
            &mut payload,
            self.dhcp_xid,
            self.mac,
            3,
            Some(self.dhcp_offered),
            Some(self.dhcp_server),
        );
        let mut frame = [0u8; MAX_FRAME];
        let length = build_udp_frame(
            &mut frame,
            [0xff; 6],
            self.mac,
            Ipv4Addr::ZERO,
            Ipv4Addr::BROADCAST,
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            &payload[..length],
            self.next_identifier,
        )?;
        self.next_identifier = self.next_identifier.wrapping_add(1);
        self.dhcp_last_action = crate::time::ticks();
        self.transmit(&frame[..length])
    }

    fn handle_dhcp(&mut self, payload: &[u8]) -> Result<(), StackError> {
        if payload.len() < 240
            || payload[0] != 2
            || read_u32(payload, 4) != Some(self.dhcp_xid)
            || payload[236..240] != [99, 130, 83, 99]
        {
            return Err(StackError::InvalidPacket);
        }
        let offered = Ipv4Addr([payload[16], payload[17], payload[18], payload[19]]);
        let mut message_type = 0;
        let mut subnet = Ipv4Addr::ZERO;
        let mut gateway = Ipv4Addr::ZERO;
        let mut dns = Ipv4Addr::ZERO;
        let mut server = Ipv4Addr::ZERO;
        let mut cursor = 240;
        while cursor < payload.len() {
            let kind = payload[cursor];
            cursor += 1;
            if kind == 0 {
                continue;
            }
            if kind == 255 {
                break;
            }
            let Some(length) = payload.get(cursor).copied().map(usize::from) else {
                return Err(StackError::InvalidPacket);
            };
            cursor += 1;
            let end = cursor
                .checked_add(length)
                .ok_or(StackError::InvalidPacket)?;
            if end > payload.len() {
                return Err(StackError::InvalidPacket);
            }
            match kind {
                1 if length == 4 => {
                    subnet = ipv4_from(payload, cursor).ok_or(StackError::InvalidPacket)?
                }
                3 if length >= 4 => {
                    gateway = ipv4_from(payload, cursor).ok_or(StackError::InvalidPacket)?
                }
                6 if length >= 4 => {
                    dns = ipv4_from(payload, cursor).ok_or(StackError::InvalidPacket)?
                }
                DHCP_MESSAGE_TYPE if length == 1 => message_type = payload[cursor],
                54 if length == 4 => {
                    server = ipv4_from(payload, cursor).ok_or(StackError::InvalidPacket)?
                }
                _ => {}
            }
            cursor = end;
        }
        match message_type {
            2 if self.dhcp == DhcpState::Discovering => {
                self.dhcp_offered = offered;
                self.dhcp_server = server;
                self.dhcp_subnet = subnet;
                self.dhcp_gateway = gateway;
                self.dhcp_dns = dns;
                self.dhcp = DhcpState::Requesting;
                self.dhcp_retries = 0;
                crate::bootlog::ok("DHCP offer received; requesting lease");
                self.send_dhcp_request()
            }
            5 if self.dhcp == DhcpState::Requesting => {
                self.ip = offered;
                self.subnet = if subnet.is_zero() {
                    Ipv4Addr([255, 255, 255, 0])
                } else {
                    subnet
                };
                self.gateway = gateway;
                self.dns_server = dns;
                self.configured = true;
                self.dhcp = DhcpState::Bound;
                crate::bootlog::ok_fmt(format_args!(
                    "DHCP lease bound ip={}.{}.{}.{} gateway={}.{}.{}.{} dns={}.{}.{}.{}",
                    self.ip.0[0],
                    self.ip.0[1],
                    self.ip.0[2],
                    self.ip.0[3],
                    self.gateway.0[0],
                    self.gateway.0[1],
                    self.gateway.0[2],
                    self.gateway.0[3],
                    self.dns_server.0[0],
                    self.dns_server.0[1],
                    self.dns_server.0[2],
                    self.dns_server.0[3],
                ));
                Ok(())
            }
            6 => {
                self.dhcp = DhcpState::Idle;
                Err(StackError::Protocol)
            }
            _ => Ok(()),
        }
    }

    fn poll_dhcp(&mut self) {
        if matches!(self.dhcp, DhcpState::Idle | DhcpState::Bound) {
            return;
        }
        let now = crate::time::ticks();
        if now.wrapping_sub(self.dhcp_last_action)
            < crate::time::scheduler_ticks() * DHCP_RETRY_SCHEDULER_TICKS
        {
            return;
        }
        if self.dhcp_retries >= DHCP_RETRY_LIMIT {
            self.dhcp = DhcpState::Idle;
            crate::bootlog::warn("DHCP lease negotiation timed out; network remains unconfigured");
            return;
        }
        self.dhcp_retries += 1;
        let result = if self.dhcp == DhcpState::Requesting {
            self.send_dhcp_request()
        } else {
            self.send_dhcp_discover()
        };
        if result.is_err() {
            self.dhcp_last_action = now;
        }
    }
}

impl Stack {
    fn send_ping(&mut self, destination: Ipv4Addr) -> Result<(), StackError> {
        let (destination_mac, _) = self.next_hop(destination)?;
        let mut icmp = [0u8; 64];
        icmp[0] = 8;
        write_u16(&mut icmp, 4, 0x4e52);
        write_u16(&mut icmp, 6, self.next_identifier);
        icmp[8..12].copy_from_slice(b"norx");
        let value = checksum(&icmp[..12]);
        write_u16(&mut icmp, 2, value);
        let mut frame = [0u8; MAX_FRAME];
        let length = build_ipv4_frame(
            &mut frame,
            destination_mac,
            self.mac,
            self.ip,
            destination,
            IP_PROTOCOL_ICMP,
            &icmp[..12],
            self.next_identifier,
        )?;
        self.next_identifier = self.next_identifier.wrapping_add(1);
        self.transmit(&frame[..length])
    }

    fn send_udp(
        &mut self,
        destination: Ipv4Addr,
        destination_port: u16,
        payload: &[u8],
    ) -> Result<(), StackError> {
        if payload.len() > MAX_UDP_PAYLOAD {
            return Err(StackError::PayloadTooLarge);
        }
        let (destination_mac, _) = self.next_hop(destination)?;
        let mut frame = [0u8; MAX_FRAME];
        let length = build_udp_frame(
            &mut frame,
            destination_mac,
            self.mac,
            self.ip,
            destination,
            self.udp.port,
            destination_port,
            payload,
            self.next_identifier,
        )?;
        self.next_identifier = self.next_identifier.wrapping_add(1);
        self.transmit(&frame[..length])
    }

    fn start_dns_query(&mut self, name: &str) -> Result<(), StackError> {
        if !self.configured || self.dns_server.is_zero() || self.dns.pending {
            return if self.dns.pending {
                Err(StackError::Busy)
            } else {
                Err(StackError::NoRoute)
            };
        }
        let mut encoded = [0u8; 64];
        let encoded_length =
            encode_dns_name(name, &mut encoded).ok_or(StackError::InvalidAddress)?;
        let id = (crate::time::ticks() as u16).wrapping_add(self.next_identifier);
        let mut payload = [0u8; 128];
        write_u16(&mut payload, 0, id);
        write_u16(&mut payload, 2, 0x0100);
        write_u16(&mut payload, 4, 1);
        payload[12..12 + encoded_length].copy_from_slice(&encoded[..encoded_length]);
        let question = 12 + encoded_length;
        write_u16(&mut payload, question, 1);
        write_u16(&mut payload, question + 2, 1);
        let (destination_mac, _) = self.next_hop(self.dns_server)?;
        let mut frame = [0u8; MAX_FRAME];
        let frame_length = build_udp_frame(
            &mut frame,
            destination_mac,
            self.mac,
            self.ip,
            self.dns_server,
            DNS_SOCKET_PORT,
            DNS_PORT,
            &payload[..question + 4],
            self.next_identifier,
        )?;
        self.transmit(&frame[..frame_length])?;
        self.dns.pending = true;
        self.dns.id = id;
        self.dns.result = None;
        self.dns.started = crate::time::ticks();
        Ok(())
    }

    fn handle_dns(&mut self, payload: &[u8]) -> Result<(), StackError> {
        if payload.len() < 12 || read_u16(payload, 0) != Some(self.dns.id) {
            return Err(StackError::InvalidPacket);
        }
        let flags = read_u16(payload, 2).ok_or(StackError::InvalidPacket)?;
        let answers = read_u16(payload, 6).ok_or(StackError::InvalidPacket)?;
        if flags & 0x8000 == 0 || flags & 0x000f != 0 {
            return Err(StackError::Protocol);
        }
        let mut cursor = skip_dns_name(payload, 12)
            .ok_or(StackError::InvalidPacket)?
            .checked_add(4)
            .ok_or(StackError::InvalidPacket)?;
        let mut answer = 0;
        while answer < answers && answer < 8 {
            cursor = skip_dns_name(payload, cursor).ok_or(StackError::InvalidPacket)?;
            if cursor.checked_add(10).ok_or(StackError::InvalidPacket)? > payload.len() {
                return Err(StackError::InvalidPacket);
            }
            let record_type = read_u16(payload, cursor).ok_or(StackError::InvalidPacket)?;
            let class = read_u16(payload, cursor + 2).ok_or(StackError::InvalidPacket)?;
            let length =
                usize::from(read_u16(payload, cursor + 8).ok_or(StackError::InvalidPacket)?);
            cursor += 10;
            if cursor
                .checked_add(length)
                .ok_or(StackError::InvalidPacket)?
                > payload.len()
            {
                return Err(StackError::InvalidPacket);
            }
            if record_type == 1 && class == 1 && length == 4 {
                let address = Ipv4Addr(
                    payload[cursor..cursor + 4]
                        .try_into()
                        .map_err(|_| StackError::InvalidPacket)?,
                );
                self.dns.result = Some(address);
                self.dns.pending = false;
                crate::bootlog::ok("DNS A record received");
                return Ok(());
            }
            cursor += length;
            answer += 1;
        }
        Err(StackError::Protocol)
    }

    fn poll_dns(&mut self) {
        if self.dns.pending
            && crate::time::ticks().wrapping_sub(self.dns.started)
                >= crate::time::scheduler_ticks() * DNS_TIMEOUT_SCHEDULER_TICKS
        {
            self.dns.pending = false;
            crate::bootlog::warn("DNS query timed out");
        }
    }

    fn start_tcp(&mut self, destination: Ipv4Addr, port: u16) -> Result<(), StackError> {
        if self.tcp.state != TcpState::Closed {
            return Err(StackError::Busy);
        }
        let (destination_mac, _) = self.next_hop(destination)?;
        let sequence = crate::time::ticks() as u32;
        let mut frame = [0u8; MAX_FRAME];
        let length = build_tcp_frame(
            &mut frame,
            destination_mac,
            self.mac,
            self.ip,
            destination,
            TCP_SOCKET_PORT,
            port,
            sequence,
            0,
            0x002,
            &[],
        )?;
        self.transmit(&frame[..length])?;
        self.tcp.remote = destination;
        self.tcp.remote_port = port;
        self.tcp.sequence = sequence;
        self.tcp.acknowledgement = 0;
        self.tcp.state = TcpState::SynSent;
        self.tcp.pending_len = 0;
        self.tcp.retries = 0;
        self.tcp.last_action = crate::time::ticks();
        Ok(())
    }

    fn send_tcp(&mut self, payload: &[u8]) -> Result<(), StackError> {
        if self.tcp.state != TcpState::Established {
            return Err(StackError::NotConnected);
        }
        if payload.len() > MAX_TCP_PAYLOAD {
            return Err(StackError::PayloadTooLarge);
        }
        if self.tcp.pending_len != 0 {
            return Err(StackError::Busy);
        }
        let (destination_mac, _) = self.next_hop(self.tcp.remote)?;
        let sequence = self.tcp.sequence;
        let mut frame = [0u8; MAX_FRAME];
        let length = build_tcp_frame(
            &mut frame,
            destination_mac,
            self.mac,
            self.ip,
            self.tcp.remote,
            TCP_SOCKET_PORT,
            self.tcp.remote_port,
            sequence,
            self.tcp.acknowledgement,
            0x018,
            payload,
        )?;
        self.transmit(&frame[..length])?;
        self.tcp.pending[..payload.len()].copy_from_slice(payload);
        self.tcp.pending_len = payload.len();
        self.tcp.pending_sequence = sequence;
        self.tcp.retries = 0;
        self.tcp.last_action = crate::time::ticks();
        Ok(())
    }

    fn send_tcp_segment(&mut self, flags: u16, payload: &[u8]) -> Result<(), StackError> {
        let (destination_mac, _) = self.next_hop(self.tcp.remote)?;
        let mut frame = [0u8; MAX_FRAME];
        let length = build_tcp_frame(
            &mut frame,
            destination_mac,
            self.mac,
            self.ip,
            self.tcp.remote,
            TCP_SOCKET_PORT,
            self.tcp.remote_port,
            self.tcp.sequence,
            self.tcp.acknowledgement,
            flags,
            payload,
        )?;
        self.transmit(&frame[..length])
    }

    fn poll_tcp(&mut self) {
        if self.tcp.state == TcpState::Closed
            || self.tcp.pending_len == 0 && self.tcp.state != TcpState::SynSent
        {
            return;
        }
        if crate::time::ticks().wrapping_sub(self.tcp.last_action)
            < crate::time::scheduler_ticks() * TCP_RETRY_SCHEDULER_TICKS
        {
            return;
        }
        if self.tcp.retries >= TCP_RETRY_LIMIT {
            self.tcp.state = TcpState::Closed;
            self.tcp.pending_len = 0;
            crate::bootlog::warn("TCP diagnostic socket timed out");
            return;
        }
        self.tcp.retries += 1;
        let result = if self.tcp.state == TcpState::SynSent {
            self.send_tcp_segment(0x002, &[])
        } else {
            let length = self.tcp.pending_len;
            let pending = self.tcp.pending;
            self.send_tcp_segment(0x018, &pending[..length])
        };
        if result.is_ok() {
            self.tcp.last_action = crate::time::ticks();
        }
    }
}

fn map_device_error(error: device::NetError) -> StackError {
    match error {
        device::NetError::Busy => StackError::Busy,
        device::NetError::WouldBlock => StackError::WouldBlock,
        device::NetError::InvalidPacket => StackError::InvalidPacket,
        _ => StackError::Device,
    }
}
