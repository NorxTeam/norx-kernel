#![allow(
    dead_code,
    reason = "Staged ABI and hardware hooks are not all enabled in the bootstrap profile"
)]

use crate::process::{ProcessTable, ThreadId, ThreadState};

pub const MAX_WAITERS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidOwner,
    InvalidWaiter,
    QueueFull,
    AlreadyWaiting,
    InvalidToken,
    NotQueueOwner,
    NotWaiterOwner,
    DeadlineOverflow,
    ValueChanged,
    ThreadState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Woken,
    Cancelled,
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitAction {
    Blocked(WaitToken),
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitToken {
    slot: u8,
    generation: u16,
    waiter: ThreadId,
}

impl WaitToken {
    pub const fn waiter(self) -> ThreadId {
        self.waiter
    }

    pub const fn generation(self) -> u16 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wake {
    pub token: WaitToken,
    pub waiter: ThreadId,
    pub status: Status,
}

#[derive(Clone, Copy)]
pub struct WakeBatch {
    entries: [Option<Wake>; MAX_WAITERS],
    length: usize,
}

impl WakeBatch {
    const fn new() -> Self {
        Self {
            entries: [None; MAX_WAITERS],
            length: 0,
        }
    }

    pub const fn len(self) -> usize {
        self.length
    }

    pub fn get(self, index: usize) -> Option<Wake> {
        (index < self.length).then(|| self.entries[index]).flatten()
    }

    fn push(&mut self, wake: Wake) {
        if self.length < MAX_WAITERS {
            self.entries[self.length] = Some(wake);
            self.length += 1;
        }
    }
}

#[derive(Clone, Copy)]
struct Waiter {
    thread: ThreadId,
    owner: u64,
    deadline: Option<u64>,
    generation: u16,
}

pub struct WaitQueue {
    owner: Option<u64>,
    waiters: [Option<Waiter>; MAX_WAITERS],
    generations: [u16; MAX_WAITERS],
    retired: [bool; MAX_WAITERS],
    completed: [Option<(u16, Status)>; MAX_WAITERS],
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            owner: None,
            waiters: [None; MAX_WAITERS],
            generations: [0; MAX_WAITERS],
            retired: [false; MAX_WAITERS],
            completed: [None; MAX_WAITERS],
        }
    }

    pub const fn new_owned(owner: u64) -> Self {
        Self {
            owner: Some(owner),
            waiters: [None; MAX_WAITERS],
            generations: [0; MAX_WAITERS],
            retired: [false; MAX_WAITERS],
            completed: [None; MAX_WAITERS],
        }
    }

    pub fn claim_owner(&mut self, owner: u64) -> Result<(), Error> {
        if owner == 0 {
            return Err(Error::InvalidOwner);
        }
        if self.owner.is_some() || self.len() != 0 {
            return Err(Error::NotQueueOwner);
        }
        self.owner = Some(owner);
        Ok(())
    }

    pub const fn owner(&self) -> Option<u64> {
        self.owner
    }

    pub fn wait(
        &mut self,
        waiter: ThreadId,
        owner: u64,
        deadline: Option<u64>,
        now: u64,
    ) -> Result<WaitAction, Error> {
        self.check_owner(owner)?;
        if waiter.get() == 0 {
            return Err(Error::InvalidWaiter);
        }
        if self.contains_thread(waiter) {
            return Err(Error::AlreadyWaiting);
        }
        if deadline.is_some_and(|deadline| deadline_reached(now, deadline)) {
            return Ok(WaitAction::TimedOut);
        }
        let slot = self
            .waiters
            .iter()
            .enumerate()
            .find(|(index, waiter)| waiter.is_none() && !self.retired[*index])
            .map(|(index, _)| index)
            .ok_or(Error::QueueFull)?;
        let generation = self.generations[slot];
        self.waiters[slot] = Some(Waiter {
            thread: waiter,
            owner,
            deadline,
            generation,
        });
        self.completed[slot] = None;
        Ok(WaitAction::Blocked(WaitToken {
            slot: slot as u8,
            generation,
            waiter,
        }))
    }

    pub fn park(
        &mut self,
        table: &mut ProcessTable,
        waiter: ThreadId,
        owner: u64,
        deadline: Option<u64>,
        now: u64,
    ) -> Result<WaitAction, Error> {
        if !matches!(
            table.thread_state(waiter),
            Ok(ThreadState::Ready | ThreadState::Running)
        ) {
            return Err(Error::ThreadState);
        }
        let action = self.wait(waiter, owner, deadline, now)?;
        let WaitAction::Blocked(token) = action else {
            return Ok(action);
        };
        if table.block_thread_until(waiter, deadline).is_err() {
            let _ = self.cancel(token, waiter);
            return Err(Error::ThreadState);
        }
        Ok(WaitAction::Blocked(token))
    }

    pub fn status(&self, token: WaitToken) -> Result<Status, Error> {
        let slot = self.token_slot(token)?;
        if let Some(waiter) = self.waiters[slot] {
            if waiter.generation == token.generation && waiter.thread == token.waiter {
                return Ok(Status::Pending);
            }
            return Err(Error::InvalidToken);
        }
        self.completed[slot]
            .filter(|(generation, _)| *generation == token.generation)
            .map(|(_, status)| status)
            .ok_or(Error::InvalidToken)
    }

    pub fn cancel(&mut self, token: WaitToken, caller: ThreadId) -> Result<Status, Error> {
        let slot = self.active_slot(token)?;
        let waiter = self.waiters[slot].ok_or(Error::InvalidToken)?;
        if waiter.thread != caller || token.waiter != caller {
            return Err(Error::NotWaiterOwner);
        }
        Ok(self.finish(slot, Status::Cancelled).status)
    }

    pub fn cancel_with_table(
        &mut self,
        table: &mut ProcessTable,
        token: WaitToken,
        caller: ThreadId,
    ) -> Result<Status, Error> {
        if !matches!(
            table.thread_state(caller),
            Ok(ThreadState::Blocked | ThreadState::Sleeping)
        ) {
            return Err(Error::ThreadState);
        }
        let status = self.cancel(token, caller)?;
        table.wake_thread(caller).map_err(|_| Error::ThreadState)?;
        Ok(status)
    }

    pub fn wake_one_owned(&mut self, owner: u64) -> Result<Option<Wake>, Error> {
        self.check_owner(owner)?;
        let Some(slot) = self.first_slot() else {
            return Ok(None);
        };
        Ok(Some(self.finish(slot, Status::Woken)))
    }

    pub fn wake_all_owned(&mut self, owner: u64) -> Result<WakeBatch, Error> {
        self.check_owner(owner)?;
        let mut wakes = WakeBatch::new();
        for slot in 0..MAX_WAITERS {
            if self.waiters[slot].is_some() {
                wakes.push(self.finish(slot, Status::Woken));
            }
        }
        Ok(wakes)
    }

    pub fn wake_one_with_table(
        &mut self,
        table: &mut ProcessTable,
        owner: u64,
    ) -> Result<Option<Wake>, Error> {
        self.check_owner(owner)?;
        for slot in 0..MAX_WAITERS {
            let Some(waiter) = self.waiters[slot] else {
                continue;
            };
            match table.thread_state(waiter.thread) {
                Ok(ThreadState::Blocked | ThreadState::Sleeping) => {
                    table
                        .wake_thread(waiter.thread)
                        .map_err(|_| Error::ThreadState)?;
                    return Ok(Some(self.finish(slot, Status::Woken)));
                }
                Ok(_) | Err(_) => {
                    self.finish(slot, Status::Cancelled);
                }
            }
        }
        Ok(None)
    }

    pub fn wake_all_with_table(
        &mut self,
        table: &mut ProcessTable,
        owner: u64,
    ) -> Result<WakeBatch, Error> {
        self.check_owner(owner)?;
        let mut wakes = WakeBatch::new();
        for slot in 0..MAX_WAITERS {
            let Some(waiter) = self.waiters[slot] else {
                continue;
            };
            match table.thread_state(waiter.thread) {
                Ok(ThreadState::Blocked | ThreadState::Sleeping) => {
                    table
                        .wake_thread(waiter.thread)
                        .map_err(|_| Error::ThreadState)?;
                    wakes.push(self.finish(slot, Status::Woken));
                }
                Ok(_) | Err(_) => {
                    self.finish(slot, Status::Cancelled);
                }
            }
        }
        Ok(wakes)
    }

    pub fn expire_owned(&mut self, owner: u64, now: u64) -> Result<WakeBatch, Error> {
        self.check_owner(owner)?;
        Ok(self.expire(now))
    }

    pub fn expire_with_table(
        &mut self,
        table: &mut ProcessTable,
        owner: u64,
        now: u64,
    ) -> Result<WakeBatch, Error> {
        self.check_owner(owner)?;
        let mut wakes = WakeBatch::new();
        for slot in 0..MAX_WAITERS {
            let Some(waiter) = self.waiters[slot] else {
                continue;
            };
            if !waiter
                .deadline
                .is_some_and(|deadline| deadline_reached(now, deadline))
            {
                continue;
            }
            match table.thread_state(waiter.thread) {
                Ok(ThreadState::Blocked | ThreadState::Sleeping) => {
                    table
                        .wake_thread(waiter.thread)
                        .map_err(|_| Error::ThreadState)?;
                    wakes.push(self.finish(slot, Status::TimedOut));
                }
                Ok(_) | Err(_) => {
                    self.finish(slot, Status::Cancelled);
                }
            }
        }
        Ok(wakes)
    }

    pub fn expire(&mut self, now: u64) -> WakeBatch {
        let mut wakes = WakeBatch::new();
        for slot in 0..MAX_WAITERS {
            let Some(waiter) = self.waiters[slot] else {
                continue;
            };
            if waiter
                .deadline
                .is_some_and(|deadline| deadline_reached(now, deadline))
            {
                wakes.push(self.finish(slot, Status::TimedOut));
            }
        }
        wakes
    }

    pub fn enqueue(&mut self, waiter: u32) -> Result<(), Error> {
        if waiter == 0 {
            return Err(Error::InvalidWaiter);
        }
        match self.wait(ThreadId::from_raw(waiter), waiter as u64, None, 0)? {
            WaitAction::Blocked(_) => Ok(()),
            WaitAction::TimedOut => Err(Error::InvalidWaiter),
        }
    }

    pub fn wake_one(&mut self) -> Option<u32> {
        let slot = self.first_slot()?;
        Some(self.finish(slot, Status::Woken).waiter.get())
    }

    pub fn wake_all(&mut self) -> usize {
        let mut count = 0;
        for slot in 0..MAX_WAITERS {
            if self.waiters[slot].is_some() {
                self.finish(slot, Status::Woken);
                count += 1;
            }
        }
        count
    }

    pub fn contains(&self, waiter: u32) -> bool {
        self.contains_thread(ThreadId::from_raw(waiter))
    }

    pub fn len(&self) -> usize {
        self.waiters
            .iter()
            .filter(|waiter| waiter.is_some())
            .count()
    }

    fn check_owner(&self, owner: u64) -> Result<(), Error> {
        if owner == 0 {
            return Err(Error::InvalidOwner);
        }
        if self.owner.is_some_and(|expected| expected != owner) {
            return Err(Error::NotQueueOwner);
        }
        Ok(())
    }

    fn contains_thread(&self, thread: ThreadId) -> bool {
        self.waiters
            .iter()
            .flatten()
            .any(|waiter| waiter.thread == thread)
    }

    fn first_slot(&self) -> Option<usize> {
        self.waiters.iter().position(Option::is_some)
    }

    fn token_slot(&self, token: WaitToken) -> Result<usize, Error> {
        let slot = token.slot as usize;
        (slot < MAX_WAITERS)
            .then_some(slot)
            .ok_or(Error::InvalidToken)
    }

    fn active_slot(&self, token: WaitToken) -> Result<usize, Error> {
        let slot = self.token_slot(token)?;
        match self.waiters[slot] {
            Some(waiter)
                if waiter.generation == token.generation && waiter.thread == token.waiter =>
            {
                Ok(slot)
            }
            _ => Err(Error::InvalidToken),
        }
    }

    fn finish(&mut self, slot: usize, status: Status) -> Wake {
        let waiter = self.waiters[slot]
            .take()
            .expect("wait queue completion without waiter");
        let token = WaitToken {
            slot: slot as u8,
            generation: waiter.generation,
            waiter: waiter.thread,
        };
        self.completed[slot] = Some((waiter.generation, status));
        if waiter.generation == u16::MAX {
            self.retired[slot] = true;
        } else {
            self.generations[slot] = waiter.generation + 1;
        }
        Wake {
            token,
            waiter: waiter.thread,
            status,
        }
    }
}

pub struct Futex {
    value: u32,
    queue: WaitQueue,
}

impl Futex {
    pub const fn new(owner: u64, value: u32) -> Self {
        Self {
            value,
            queue: WaitQueue::new_owned(owner),
        }
    }

    pub const fn load(&self) -> u32 {
        self.value
    }

    pub fn store(&mut self, owner: u64, value: u32) -> Result<(), Error> {
        self.queue.check_owner(owner)?;
        self.value = value;
        Ok(())
    }

    pub fn wait(
        &mut self,
        waiter: ThreadId,
        owner: u64,
        expected: u32,
        deadline: Option<u64>,
        now: u64,
    ) -> Result<WaitAction, Error> {
        self.queue.check_owner(owner)?;
        if self.value != expected {
            return Err(Error::ValueChanged);
        }
        self.queue.wait(waiter, owner, deadline, now)
    }

    pub fn wait_with_table(
        &mut self,
        table: &mut ProcessTable,
        waiter: ThreadId,
        owner: u64,
        expected: u32,
        deadline: Option<u64>,
        now: u64,
    ) -> Result<WaitAction, Error> {
        self.queue.check_owner(owner)?;
        if self.value != expected {
            return Err(Error::ValueChanged);
        }
        self.queue.park(table, waiter, owner, deadline, now)
    }

    pub fn wake_one_owned(&mut self, owner: u64) -> Result<Option<Wake>, Error> {
        self.queue.wake_one_owned(owner)
    }

    pub fn wake_all_owned(&mut self, owner: u64) -> Result<WakeBatch, Error> {
        self.queue.wake_all_owned(owner)
    }

    pub fn wake_one_with_table(
        &mut self,
        table: &mut ProcessTable,
        owner: u64,
    ) -> Result<Option<Wake>, Error> {
        self.queue.wake_one_with_table(table, owner)
    }

    pub fn wake_all_with_table(
        &mut self,
        table: &mut ProcessTable,
        owner: u64,
    ) -> Result<WakeBatch, Error> {
        self.queue.wake_all_with_table(table, owner)
    }

    pub fn queue(&self) -> &WaitQueue {
        &self.queue
    }

    pub fn queue_mut(&mut self) -> &mut WaitQueue {
        &mut self.queue
    }
}

pub fn deadline_after(now: u64, timeout: u64) -> Result<u64, Error> {
    if timeout > u64::MAX / 2 {
        return Err(Error::DeadlineOverflow);
    }
    now.checked_add(timeout).ok_or(Error::DeadlineOverflow)
}

pub const fn deadline_reached(now: u64, deadline: u64) -> bool {
    now.wrapping_sub(deadline) < (1u64 << 63)
}

pub fn contract_self_check() {
    let owner = 7;
    let first_thread = ThreadId::from_raw(1);
    let second_thread = ThreadId::from_raw(2);
    let mut queue = WaitQueue::new_owned(owner);
    let first = match queue.wait(first_thread, owner, Some(20), 10) {
        Ok(WaitAction::Blocked(token)) => token,
        _ => panic!("wait queue did not block"),
    };
    assert_eq!(queue.status(first), Ok(Status::Pending));
    assert_eq!(
        queue.wait(first_thread, owner, None, 10),
        Err(Error::AlreadyWaiting)
    );
    assert_eq!(queue.wake_one_owned(owner + 1), Err(Error::NotQueueOwner));
    let wake = queue.wake_one_owned(owner).unwrap().unwrap();
    assert_eq!(wake.waiter, first_thread);
    assert_eq!(wake.status, Status::Woken);
    assert_eq!(queue.status(first), Ok(Status::Woken));

    let cancelled = match queue.wait(second_thread, owner, None, 10) {
        Ok(WaitAction::Blocked(token)) => token,
        _ => panic!("second wait did not block"),
    };
    assert_eq!(
        queue.cancel(cancelled, first_thread),
        Err(Error::NotWaiterOwner)
    );
    assert_eq!(
        queue.cancel(cancelled, second_thread),
        Ok(Status::Cancelled)
    );
    assert_eq!(queue.status(cancelled), Ok(Status::Cancelled));

    let reused = match queue.wait(ThreadId::from_raw(3), owner, None, 10) {
        Ok(WaitAction::Blocked(token)) => token,
        _ => panic!("reused wait did not block"),
    };
    assert_eq!(queue.status(cancelled), Err(Error::InvalidToken));
    assert_ne!(cancelled.generation(), reused.generation());
    let batch = queue.wake_all_owned(owner).unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(queue.status(reused), Ok(Status::Woken));

    let timed = match queue.wait(ThreadId::from_raw(4), owner, Some(30), 10) {
        Ok(WaitAction::Blocked(token)) => token,
        _ => panic!("timed wait did not block"),
    };
    assert_eq!(queue.expire_owned(owner, 29).unwrap().len(), 0);
    let timeout = queue.expire_owned(owner, 30).unwrap();
    assert_eq!(timeout.len(), 1);
    assert_eq!(timeout.get(0).unwrap().token, timed);
    assert_eq!(queue.status(timed), Ok(Status::TimedOut));
    assert_eq!(
        queue.wait(ThreadId::from_raw(5), owner, Some(10), 10),
        Ok(WaitAction::TimedOut)
    );
    assert_eq!(deadline_after(10, u64::MAX), Err(Error::DeadlineOverflow));
    assert!(deadline_reached(0, u64::MAX));
    assert!(!deadline_reached(u64::MAX, 0));

    let mut futex = Futex::new(11, 3);
    assert_eq!(
        futex.wait(ThreadId::from_raw(6), 11, 4, None, 0),
        Err(Error::ValueChanged)
    );
    let futex_token = match futex.wait(ThreadId::from_raw(6), 11, 3, None, 0) {
        Ok(WaitAction::Blocked(token)) => token,
        _ => panic!("futex did not block"),
    };
    futex.store(11, 4).unwrap();
    assert_eq!(
        futex.wake_one_owned(11).unwrap().unwrap().token,
        futex_token
    );
    assert_eq!(futex.store(12, 5), Err(Error::NotQueueOwner));

    let mut table = ProcessTable::new();
    let (_, init_thread) = table
        .create_init()
        .unwrap_or_else(|_| panic!("init thread allocation"));
    let (_, blocked_thread) = table
        .spawn_child(
            crate::process::ProcessId::INIT,
            crate::process::Credentials::BOOTSTRAP,
        )
        .unwrap_or_else(|_| panic!("blocked thread allocation"));
    table
        .switch_to(None, init_thread)
        .unwrap_or_else(|_| panic!("init scheduling"));
    let mut parked = WaitQueue::new_owned(19);
    let parked_token = match parked
        .park(&mut table, blocked_thread, 19, Some(40), 10)
        .unwrap()
    {
        WaitAction::Blocked(token) => token,
        WaitAction::TimedOut => panic!("park timed out"),
    };
    assert_eq!(table.thread_state(blocked_thread), Ok(ThreadState::Blocked));
    let parked_wake = parked.wake_one_with_table(&mut table, 19).unwrap().unwrap();
    assert_eq!(parked_wake.token, parked_token);
    assert_eq!(table.thread_state(blocked_thread), Ok(ThreadState::Ready));
    let timed_thread = table
        .spawn_kernel_thread()
        .unwrap_or_else(|_| panic!("timed thread allocation"));
    let timed_token = match parked
        .park(&mut table, timed_thread, 19, Some(50), 10)
        .unwrap()
    {
        WaitAction::Blocked(token) => token,
        WaitAction::TimedOut => panic!("table park timed out"),
    };
    let expired = parked.expire_with_table(&mut table, 19, 50).unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired.get(0).unwrap().token, timed_token);
    assert_eq!(table.thread_state(timed_thread), Ok(ThreadState::Ready));
}
