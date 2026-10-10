#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

//! Bounded POSIX signal state.
//!
//! This module contains no scheduler, address-space, or user-pointer code.
//! Signal generation only records state.  A caller at a safe boundary selects
//! a pending signal and either applies its default action or installs a
//! kernel-owned delivery frame.  User-stack serialization and architecture
//! register installation remain the responsibility of that boundary.

pub const MAX_SIGNALS: usize = 64;
pub const SIGNAL_MASK: u64 = u64::MAX << 1;
pub const MAX_SIGNAL_FRAMES: usize = 4;
pub const SIGNAL_FRAME_SIZE: u64 = 512;
pub const SIGNAL_STACK_ALIGNMENT: u64 = 16;

pub const SIGHUP: u8 = 1;
pub const SIGINT: u8 = 2;
pub const SIGQUIT: u8 = 3;
pub const SIGILL: u8 = 4;
pub const SIGTRAP: u8 = 5;
pub const SIGABRT: u8 = 6;
pub const SIGBUS: u8 = 7;
pub const SIGFPE: u8 = 8;
pub const SIGKILL: u8 = 9;
pub const SIGUSR1: u8 = 10;
pub const SIGSEGV: u8 = 11;
pub const SIGUSR2: u8 = 12;
pub const SIGPIPE: u8 = 13;
pub const SIGALRM: u8 = 14;
pub const SIGTERM: u8 = 15;
pub const SIGSTKFLT: u8 = 16;
pub const SIGCHLD: u8 = 17;
pub const SIGCONT: u8 = 18;
pub const SIGSTOP: u8 = 19;
pub const SIGTSTP: u8 = 20;
pub const SIGTTIN: u8 = 21;
pub const SIGTTOU: u8 = 22;
pub const SIGURG: u8 = 23;
pub const SIGXCPU: u8 = 24;
pub const SIGXFSZ: u8 = 25;
pub const SIGVTALRM: u8 = 26;
pub const SIGPROF: u8 = 27;
pub const SIGWINCH: u8 = 28;
pub const SIGIO: u8 = 29;
pub const SIGPWR: u8 = 30;
pub const SIGSYS: u8 = 31;

pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;

pub const SA_RESTART: u32 = 1 << 0;
pub const SA_NOCLDSTOP: u32 = 1 << 1;
pub const SA_NOCLDWAIT: u32 = 1 << 2;
pub const SA_NODEFER: u32 = 1 << 3;
pub const SA_RESETHAND: u32 = 1 << 4;
pub const SA_SUPPORTED: u32 = SA_RESTART | SA_NOCLDSTOP | SA_NOCLDWAIT | SA_NODEFER | SA_RESETHAND;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalError {
    InvalidSignal,
    InvalidMask,
    InvalidHandler,
    Uncatchable,
    UnsupportedFlags,
    FrameCapacity,
    FrameUnavailable,
    StaleFrame,
    TokenExhausted,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultAction {
    Ignore,
    Terminate,
    Stop,
    Continue,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalAction {
    /// `SIG_DFL`, `SIG_IGN`, or a user handler address.  This is not a
    /// capability or a proof that the address is mapped/executable.
    pub handler: u64,
    pub mask: u64,
    pub flags: u32,
    pub reserved: u32,
}

impl SignalAction {
    pub const DEFAULT: Self = Self {
        handler: SIG_DFL,
        mask: 0,
        flags: 0,
        reserved: 0,
    };

    pub const IGNORE: Self = Self {
        handler: SIG_IGN,
        mask: 0,
        flags: 0,
        reserved: 0,
    };

    pub const fn handler(handler: u64, mask: u64, flags: u32) -> Self {
        Self {
            handler,
            mask,
            flags,
            reserved: 0,
        }
    }

    pub const fn is_default(self) -> bool {
        self.handler == SIG_DFL
    }

    pub const fn is_ignored(self) -> bool {
        self.handler == SIG_IGN
    }

    pub fn validate(self, signal: u8) -> Result<(), SignalError> {
        if !is_valid_signal(signal) {
            return Err(SignalError::InvalidSignal);
        }
        if self.reserved != 0 || self.mask & !SIGNAL_MASK != 0 {
            return Err(SignalError::InvalidMask);
        }
        if self.flags & !SA_SUPPORTED != 0
            || ((self.flags & (SA_NOCLDSTOP | SA_NOCLDWAIT)) != 0 && signal != SIGCHLD)
        {
            return Err(SignalError::UnsupportedFlags);
        }
        if (signal == SIGKILL || signal == SIGSTOP) && self != Self::DEFAULT {
            return Err(SignalError::Uncatchable);
        }
        Ok(())
    }
}

pub const fn is_valid_signal(signal: u8) -> bool {
    signal != 0 && (signal as usize) < MAX_SIGNALS
}

pub const fn signal_bit(signal: u8) -> u64 {
    if is_valid_signal(signal) {
        1u64 << signal
    } else {
        0
    }
}

pub const fn default_action(signal: u8) -> DefaultAction {
    match signal {
        SIGCHLD | SIGURG | SIGWINCH => DefaultAction::Ignore,
        SIGCONT => DefaultAction::Continue,
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => DefaultAction::Stop,
        _ => DefaultAction::Terminate,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalState {
    dispositions: [SignalAction; MAX_SIGNALS],
    pending: u64,
}

impl SignalState {
    pub const fn new() -> Self {
        Self {
            dispositions: [SignalAction::DEFAULT; MAX_SIGNALS],
            pending: 0,
        }
    }

    pub const fn inherit(self) -> Self {
        Self {
            dispositions: self.dispositions,
            pending: 0,
        }
    }

    pub fn action(&self, signal: u8) -> Result<SignalAction, SignalError> {
        if !is_valid_signal(signal) {
            return Err(SignalError::InvalidSignal);
        }
        Ok(self.dispositions[signal as usize])
    }

    pub fn install(
        &mut self,
        signal: u8,
        action: SignalAction,
    ) -> Result<SignalAction, SignalError> {
        action.validate(signal)?;
        let slot = signal as usize;
        let old = self.dispositions[slot];
        self.dispositions[slot] = action;
        if action.is_ignored() {
            self.pending &= !signal_bit(signal);
        }
        Ok(old)
    }

    pub fn pending(&self, signal: u8) -> Result<bool, SignalError> {
        if !is_valid_signal(signal) {
            return Err(SignalError::InvalidSignal);
        }
        Ok(self.pending & signal_bit(signal) != 0)
    }

    pub fn set_pending(&mut self, signal: u8) -> Result<(), SignalError> {
        if signal == 0 {
            return Ok(());
        }
        if !is_valid_signal(signal) {
            return Err(SignalError::InvalidSignal);
        }
        self.pending |= signal_bit(signal);
        Ok(())
    }

    pub fn clear_pending(&mut self, signal: u8) {
        self.pending &= !signal_bit(signal);
    }

    pub const fn pending_mask(&self) -> u64 {
        self.pending
    }

    pub fn clear_all(&mut self) {
        self.pending = 0;
        self.dispositions = [SignalAction::DEFAULT; MAX_SIGNALS];
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserSignalContext {
    pub registers: [u64; 32],
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub flags: u64,
}

impl UserSignalContext {
    pub const EMPTY: Self = Self {
        registers: [0; 32],
        instruction_pointer: 0,
        stack_pointer: 0,
        flags: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestartState {
    pub syscall: u64,
    pub arguments: [u64; 6],
    pub instruction_pointer: u64,
    pub restartable: bool,
    pub partial_result: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalFrame {
    pub token: u32,
    pub signal: u8,
    pub frame_pointer: u64,
    pub saved_mask: u64,
    pub saved_context: UserSignalContext,
    pub restart: Option<RestartState>,
    pub interrupted: Option<RestartState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Bounded signal delivery owns its frame without a kernel heap"
)]
pub enum SignalDelivery {
    NoPending,
    Ignored {
        signal: u8,
    },
    Default {
        signal: u8,
        action: DefaultAction,
    },
    Handler {
        signal: u8,
        frame: SignalFrame,
        context: UserSignalContext,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignalReturn {
    pub context: UserSignalContext,
    pub restart: Option<RestartState>,
    pub interrupted: Option<RestartState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadSignalState {
    mask: u64,
    pending: u64,
    frames: [Option<SignalFrame>; MAX_SIGNAL_FRAMES],
    frame_count: usize,
    next_token: u32,
    restart: Option<RestartState>,
}

impl ThreadSignalState {
    pub const fn new() -> Self {
        Self {
            mask: 0,
            pending: 0,
            frames: [None; MAX_SIGNAL_FRAMES],
            frame_count: 0,
            next_token: 0,
            restart: None,
        }
    }

    pub const fn inherit(self) -> Self {
        Self {
            mask: self.mask,
            pending: 0,
            frames: [None; MAX_SIGNAL_FRAMES],
            frame_count: 0,
            next_token: 0,
            restart: None,
        }
    }

    pub const fn mask(&self) -> u64 {
        self.mask
    }

    pub fn set_mask(&mut self, mask: u64) -> Result<u64, SignalError> {
        if mask & !SIGNAL_MASK != 0 {
            return Err(SignalError::InvalidMask);
        }
        let old = self.mask;
        self.mask = mask & !(signal_bit(SIGKILL) | signal_bit(SIGSTOP));
        Ok(old)
    }

    pub fn change_mask(&mut self, how: MaskHow, mask: u64) -> Result<u64, SignalError> {
        if mask & !SIGNAL_MASK != 0 {
            return Err(SignalError::InvalidMask);
        }
        let old = self.mask;
        let unmaskable = signal_bit(SIGKILL) | signal_bit(SIGSTOP);
        self.mask = match how {
            MaskHow::Block => self.mask | mask,
            MaskHow::Unblock => self.mask & !mask,
            MaskHow::Set => mask,
        } & !unmaskable;
        Ok(old)
    }

    pub fn pending(&self, signal: u8) -> Result<bool, SignalError> {
        if !is_valid_signal(signal) {
            return Err(SignalError::InvalidSignal);
        }
        Ok(self.pending & signal_bit(signal) != 0)
    }

    pub fn pending_mask(&self) -> u64 {
        self.pending
    }

    pub fn set_pending(&mut self, signal: u8) -> Result<(), SignalError> {
        if signal == 0 {
            return Ok(());
        }
        if !is_valid_signal(signal) {
            return Err(SignalError::InvalidSignal);
        }
        self.pending |= signal_bit(signal);
        Ok(())
    }

    pub fn clear_pending(&mut self, signal: u8) {
        self.pending &= !signal_bit(signal);
    }

    pub fn clear_pending_mask(&mut self, mask: u64) {
        self.pending &= !mask;
    }

    pub fn set_restart(&mut self, restart: Option<RestartState>) {
        self.restart = restart;
    }

    pub const fn restart(&self) -> Option<RestartState> {
        self.restart
    }

    pub fn clear_for_teardown(&mut self) {
        self.mask = 0;
        self.pending = 0;
        self.frames = [None; MAX_SIGNAL_FRAMES];
        self.frame_count = 0;
        self.restart = None;
    }

    pub fn begin_handler(
        &mut self,
        signal: u8,
        action: SignalAction,
        context: UserSignalContext,
    ) -> Result<SignalDelivery, SignalError> {
        action.validate(signal)?;
        if action.is_default() || action.is_ignored() {
            return Err(SignalError::InvalidHandler);
        }
        if self.frame_count == MAX_SIGNAL_FRAMES {
            return Err(SignalError::FrameCapacity);
        }
        let frame_pointer = context
            .stack_pointer
            .checked_sub(SIGNAL_FRAME_SIZE)
            .map(|pointer| pointer & !(SIGNAL_STACK_ALIGNMENT - 1))
            .filter(|pointer| *pointer != 0)
            .ok_or(SignalError::FrameUnavailable)?;
        let token = self
            .next_token
            .checked_add(1)
            .filter(|token| *token != 0)
            .ok_or(SignalError::TokenExhausted)?;

        let restart = self.restart.take();
        let (restart_for_frame, interrupted) = match restart {
            Some(restart)
                if restart.restartable
                    && !restart.partial_result
                    && action.flags & SA_RESTART != 0 =>
            {
                (Some(restart), None)
            }
            Some(restart) => (None, Some(restart)),
            None => (None, None),
        };
        let saved_mask = self.mask;
        let mut next_mask = self.mask | action.mask;
        if action.flags & SA_NODEFER == 0 {
            next_mask |= signal_bit(signal);
        }
        self.mask = next_mask & SIGNAL_MASK & !(signal_bit(SIGKILL) | signal_bit(SIGSTOP));
        self.next_token = token;

        let frame = SignalFrame {
            token,
            signal,
            frame_pointer,
            saved_mask,
            saved_context: context,
            restart: restart_for_frame,
            interrupted,
        };
        self.frames[self.frame_count] = Some(frame);
        self.frame_count += 1;

        let mut next = context;
        next.instruction_pointer = action.handler;
        next.stack_pointer = frame_pointer;
        next.registers[0] = signal as u64;
        next.registers[1] = token as u64;
        Ok(SignalDelivery::Handler {
            signal,
            frame,
            context: next,
        })
    }

    pub fn finish_handler(&mut self, token: u32) -> Result<SignalReturn, SignalError> {
        if self.frame_count == 0 {
            return Err(SignalError::StaleFrame);
        }
        let index = self.frame_count - 1;
        let frame = self.frames[index].ok_or(SignalError::StaleFrame)?;
        if frame.token != token {
            return Err(SignalError::StaleFrame);
        }
        self.frames[index] = None;
        self.frame_count = index;
        self.mask = frame.saved_mask & SIGNAL_MASK & !(signal_bit(SIGKILL) | signal_bit(SIGSTOP));
        let mut context = frame.saved_context;
        if let Some(restart) = frame.restart {
            context.instruction_pointer = restart.instruction_pointer;
        }
        Ok(SignalReturn {
            context,
            restart: frame.restart,
            interrupted: frame.interrupted,
        })
    }

    pub const fn frame_count(&self) -> usize {
        self.frame_count
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskHow {
    Block,
    Unblock,
    Set,
}

pub fn contract_self_check() {
    assert_eq!(signal_bit(0), 0);
    assert_eq!(default_action(SIGCHLD), DefaultAction::Ignore);
    assert_eq!(default_action(SIGSTOP), DefaultAction::Stop);
    assert_eq!(default_action(SIGKILL), DefaultAction::Terminate);
    assert!(SignalAction::handler(0x1000, 0, SA_RESTART)
        .validate(SIGUSR1)
        .is_ok());
    assert_eq!(
        SignalAction::handler(0x1000, 0, 0).validate(SIGKILL),
        Err(SignalError::Uncatchable)
    );
    assert_eq!(
        SignalAction::handler(0x1000, signal_bit(SIGUSR1), 0).validate(SIGUSR1),
        Ok(())
    );

    let mut process = SignalState::new();
    process.set_pending(SIGUSR1).unwrap();
    assert!(process.pending(SIGUSR1).unwrap());
    process.install(SIGUSR1, SignalAction::IGNORE).unwrap();
    assert!(!process.pending(SIGUSR1).unwrap());

    let mut thread = ThreadSignalState::new();
    thread.set_restart(Some(RestartState {
        syscall: 35,
        arguments: [7, 0, 0, 0, 0, 0],
        instruction_pointer: 0x4000,
        restartable: true,
        partial_result: false,
    }));
    let delivery = thread
        .begin_handler(
            SIGUSR1,
            SignalAction::handler(0x5000, signal_bit(SIGTERM), SA_RESTART),
            UserSignalContext {
                stack_pointer: 0x9000,
                instruction_pointer: 0x4000,
                ..UserSignalContext::EMPTY
            },
        )
        .unwrap();
    let (token, context) = match delivery {
        SignalDelivery::Handler { frame, context, .. } => (frame.token, context),
        _ => panic!("handler delivery missing"),
    };
    assert_eq!(context.instruction_pointer, 0x5000);
    assert_eq!(context.stack_pointer, 0x8e00);
    assert_eq!(thread.mask(), signal_bit(SIGTERM) | signal_bit(SIGUSR1));
    let returned = thread.finish_handler(token).unwrap();
    assert_eq!(returned.restart.unwrap().syscall, 35);
    assert_eq!(returned.context.instruction_pointer, 0x4000);
    assert_eq!(thread.frame_count(), 0);
    assert_eq!(thread.finish_handler(token), Err(SignalError::StaleFrame));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_are_bounded_and_unmaskable_signals_stay_unblocked() {
        let mut state = ThreadSignalState::new();
        assert_eq!(
            state.set_mask(signal_bit(SIGKILL) | signal_bit(SIGSTOP) | signal_bit(SIGINT)),
            Ok(0)
        );
        assert_eq!(state.mask(), signal_bit(SIGINT));
        assert_eq!(state.set_mask(1), Err(SignalError::InvalidMask));
    }

    #[test]
    fn restart_is_denied_after_partial_result_or_without_sa_restart() {
        let mut state = ThreadSignalState::new();
        state.set_restart(Some(RestartState {
            syscall: 0,
            arguments: [0; 6],
            instruction_pointer: 1,
            restartable: true,
            partial_result: true,
        }));
        let delivery = state
            .begin_handler(
                SIGUSR1,
                SignalAction::handler(2, 0, SA_RESTART),
                UserSignalContext {
                    stack_pointer: 0x1000,
                    ..UserSignalContext::EMPTY
                },
            )
            .unwrap();
        let token = match delivery {
            SignalDelivery::Handler { frame, .. } => frame.token,
            _ => unreachable!(),
        };
        let returned = state.finish_handler(token).unwrap();
        assert!(returned.restart.is_none());
        assert!(returned.interrupted.is_some());
    }
}
