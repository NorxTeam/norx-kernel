use crate::boot::{PixelFormat, RawFramebuffer};

const MAX_MODES: usize = 8;
const MAX_DAMAGE: usize = 32;
pub const MODE_CONTRACT_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModePolicy {
    FirmwareFixed,
    ControllerTransactional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeContract {
    pub version: u8,
    pub firmware_policy: ModePolicy,
    pub host_window_resize_changes_guest_mode: bool,
    pub scanout_resize_changes_guest_mode: bool,
}

pub const MODE_CONTRACT: ModeContract = ModeContract {
    version: MODE_CONTRACT_VERSION,
    firmware_policy: ModePolicy::FirmwareFixed,
    host_window_resize_changes_guest_mode: false,
    scanout_resize_changes_guest_mode: false,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported,
    NoDisplay,
    InvalidMode,
    InvalidRegion,
    DamageFull,
    BufferTooSmall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitResult {
    Ready(Status),
    Unsupported,
    Failed(Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    pub id: u16,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub bytes_per_pixel: usize,
    pub format: PixelFormat,
    pub policy: ModePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub visible: bool,
}

impl Cursor {
    const HIDDEN: Self = Self {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        visible: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub contract_version: u8,
    pub mode_policy: ModePolicy,
    pub present: bool,
    pub active_mode: u16,
    pub mode_count: u8,
    pub edid_available: bool,
    pub pending_damage: u8,
    pub cursor_visible: bool,
    pub hotplug_generation: u64,
    pub flushes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotplugEvent {
    Connected,
    Disconnected,
}

#[derive(Clone, Copy)]
struct State {
    modes: [Mode; MAX_MODES],
    mode_count: usize,
    active_mode: u16,
    damage: [DamageRegion; MAX_DAMAGE],
    damage_len: usize,
    cursor: Cursor,
    hotplug_generation: u64,
    flushes: u64,
}

static mut STATE: Option<State> = None;

pub fn contract_self_check() {
    assert_eq!(MODE_CONTRACT.version, MODE_CONTRACT_VERSION);
    assert_eq!(MODE_CONTRACT.firmware_policy, ModePolicy::FirmwareFixed);
    const {
        assert!(!MODE_CONTRACT.host_window_resize_changes_guest_mode);
        assert!(!MODE_CONTRACT.scanout_resize_changes_guest_mode);
    }
    let _ = ModePolicy::ControllerTransactional;
    let _ = HotplugEvent::Connected;
    let _ = HotplugEvent::Disconnected;
    assert!(valid_geometry(640, 480, 2560, 4, 2560 * 480));
    assert!(!valid_geometry(640, 480, 100, 4, 2560 * 480));
    assert!(!valid_geometry(640, 480, 2560, 4, 1024));
    assert_eq!(validate_scanout(800, 600, 1200, 800), Some((800, 600)));
    assert!(validate_scanout(800, 600, 640, 480).is_none());
    assert!(valid_region(
        DamageRegion {
            x: 10,
            y: 10,
            width: 20,
            height: 20,
        },
        640,
        480,
    ));
    assert!(!valid_region(
        DamageRegion {
            x: 630,
            y: 10,
            width: 20,
            height: 20,
        },
        640,
        480,
    ));
}

pub fn init(raw: Option<RawFramebuffer>) -> InitResult {
    let Some(raw) = raw else {
        unsafe { core::ptr::addr_of_mut!(STATE).write(None) };
        crate::bootlog::start(3, "discovering display framebuffer");
        crate::bootlog::warn("display framebuffer unavailable; serial remains active");
        return InitResult::Unsupported;
    };
    let Some(mode) = mode_from_raw(raw) else {
        unsafe { core::ptr::addr_of_mut!(STATE).write(None) };
        crate::bootlog::start(3, "discovering display framebuffer");
        crate::bootlog::warn("display framebuffer descriptor rejected; display deferred");
        return InitResult::Failed(Error::InvalidMode);
    };
    let state = State {
        modes: [mode; MAX_MODES],
        mode_count: 1,
        active_mode: mode.id,
        damage: [DamageRegion {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }; MAX_DAMAGE],
        damage_len: 0,
        cursor: Cursor::HIDDEN,
        hotplug_generation: 0,
        flushes: 0,
    };
    let status = state.status();
    unsafe { core::ptr::addr_of_mut!(STATE).write(Some(state)) };
    crate::bootlog::start(3, "discovering display framebuffer");
    crate::bootlog::ok_fmt(format_args!(
        "display ready mode={} {}x{} pitch={} bpp={} modes=1 edid=false damage={}",
        mode.id, mode.width, mode.height, mode.stride, mode.bytes_per_pixel, MAX_DAMAGE
    ));
    InitResult::Ready(status)
}

pub fn status() -> Option<Status> {
    unsafe { (&*core::ptr::addr_of!(STATE)).as_ref().map(State::status) }
}

pub fn modes(output: &mut [Mode]) -> Result<usize, Error> {
    unsafe {
        let Some(state) = (&*core::ptr::addr_of!(STATE)).as_ref() else {
            return Err(Error::NoDisplay);
        };
        if output.len() < state.mode_count {
            return Err(Error::BufferTooSmall);
        }
        output[..state.mode_count].copy_from_slice(&state.modes[..state.mode_count]);
        Ok(state.mode_count)
    }
}

pub fn set_mode(id: u16) -> Result<Mode, Error> {
    unsafe {
        let Some(state) = (&mut *core::ptr::addr_of_mut!(STATE)).as_mut() else {
            return Err(Error::NoDisplay);
        };
        let Some(mode) = state.modes[..state.mode_count]
            .iter()
            .copied()
            .find(|mode| mode.id == id)
        else {
            return Err(Error::InvalidMode);
        };
        if id != state.active_mode {
            return Err(Error::Unsupported);
        }
        Ok(mode)
    }
}

pub fn edid(output: &mut [u8]) -> Result<usize, Error> {
    unsafe {
        if (&*core::ptr::addr_of!(STATE)).is_none() {
            return Err(Error::NoDisplay);
        }
    }
    let _ = output;
    Err(Error::Unsupported)
}

pub fn queue_damage(region: DamageRegion) -> Result<(), Error> {
    unsafe {
        let Some(state) = (&mut *core::ptr::addr_of_mut!(STATE)).as_mut() else {
            return Err(Error::NoDisplay);
        };
        let Some(mode) = state.active_mode() else {
            return Err(Error::InvalidMode);
        };
        if !valid_region(region, mode.width, mode.height) {
            return Err(Error::InvalidRegion);
        }
        if state.damage_len == MAX_DAMAGE {
            return Err(Error::DamageFull);
        }
        state.damage[state.damage_len] = region;
        state.damage_len += 1;
        Ok(())
    }
}

pub fn flush() -> Result<usize, Error> {
    unsafe {
        let Some(state) = (&mut *core::ptr::addr_of_mut!(STATE)).as_mut() else {
            return Err(Error::NoDisplay);
        };
        let flushed = state.damage_len;
        state.damage_len = 0;
        state.flushes = state.flushes.saturating_add(1);
        Ok(flushed)
    }
}

pub fn set_cursor(cursor: Cursor) -> Result<(), Error> {
    unsafe {
        let Some(state) = (&mut *core::ptr::addr_of_mut!(STATE)).as_mut() else {
            return Err(Error::NoDisplay);
        };
        let Some(mode) = state.active_mode() else {
            return Err(Error::InvalidMode);
        };
        if cursor.visible
            && !valid_region(
                DamageRegion {
                    x: cursor.x,
                    y: cursor.y,
                    width: cursor.width,
                    height: cursor.height,
                },
                mode.width,
                mode.height,
            )
        {
            return Err(Error::InvalidRegion);
        }
        state.cursor = cursor;
        Ok(())
    }
}

pub fn poll_hotplug() -> Option<HotplugEvent> {
    None
}

impl State {
    fn active_mode(&self) -> Option<Mode> {
        self.modes[..self.mode_count]
            .iter()
            .copied()
            .find(|mode| mode.id == self.active_mode)
    }

    fn status(&self) -> Status {
        Status {
            contract_version: MODE_CONTRACT.version,
            mode_policy: self
                .active_mode()
                .map_or(MODE_CONTRACT.firmware_policy, |mode| mode.policy),
            present: true,
            active_mode: self.active_mode,
            mode_count: self.mode_count as u8,
            edid_available: false,
            pending_damage: self.damage_len as u8,
            cursor_visible: self.cursor.visible,
            hotplug_generation: self.hotplug_generation,
            flushes: self.flushes,
        }
    }
}

fn mode_from_raw(raw: RawFramebuffer) -> Option<Mode> {
    valid_geometry(
        raw.width,
        raw.height,
        raw.stride,
        raw.bytes_per_pixel,
        raw.size,
    )
    .then_some(Mode {
        id: 0,
        width: raw.width,
        height: raw.height,
        stride: raw.stride,
        bytes_per_pixel: raw.bytes_per_pixel,
        format: raw.format,
        policy: MODE_CONTRACT.firmware_policy,
    })
}

fn valid_geometry(
    width: u32,
    height: u32,
    stride: usize,
    bytes_per_pixel: usize,
    size: usize,
) -> bool {
    width != 0
        && height != 0
        && matches!(bytes_per_pixel, 3 | 4)
        && stride >= width as usize * bytes_per_pixel
        && stride
            .checked_mul(height as usize)
            .is_some_and(|end| end <= size)
}

fn valid_region(region: DamageRegion, width: u32, height: u32) -> bool {
    region.width != 0
        && region.height != 0
        && region
            .x
            .checked_add(region.width)
            .is_some_and(|end| end <= width)
        && region
            .y
            .checked_add(region.height)
            .is_some_and(|end| end <= height)
}

pub fn validate_scanout(
    guest_width: u32,
    guest_height: u32,
    scanout_width: u32,
    scanout_height: u32,
) -> Option<(u32, u32)> {
    (guest_width != 0
        && guest_height != 0
        && scanout_width >= guest_width
        && scanout_height >= guest_height)
        .then_some((guest_width, guest_height))
}
