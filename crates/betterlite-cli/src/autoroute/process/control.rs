use std::time::{Duration, Instant};

use crate::sso_polling::polling::{Sleeper as _, ThreadSleeper};

pub const TERMINATION_GRACE_PERIOD: Duration = Duration::from_secs(5);
pub const TERMINATION_POLL_INTERVAL: Duration = Duration::from_millis(200);

pub trait Clock {
    fn now(&self) -> Instant;

    fn sleep(&self, duration: Duration);
}

pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        ThreadSleeper.sleep(duration);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalOutcome {
    Exists,
    Missing,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermSignal {
    Term,
    Kill,
}

pub trait SignalPort {
    fn probe(&self, pid: u32) -> SignalOutcome;

    fn send(&self, pid: u32, signal: TermSignal);

    // LiteLLM counts a permission error as the process existing under another user
    fn is_running(&self, pid: u32) -> bool {
        !matches!(self.probe(pid), SignalOutcome::Missing)
    }
}

// LiteLLM sends SIGTERM, polls for the grace period, then escalates to SIGKILL if it is still there
pub fn terminate(pid: u32, port: &dyn SignalPort, clock: &dyn Clock) {
    if !port.is_running(pid) {
        return;
    }
    port.send(pid, TermSignal::Term);
    let deadline = clock.now().checked_add(TERMINATION_GRACE_PERIOD);
    while deadline.is_some_and(|end| clock.now() < end) && port.is_running(pid) {
        clock.sleep(TERMINATION_POLL_INTERVAL);
    }
    if port.is_running(pid) {
        port.send(pid, TermSignal::Kill);
    }
}

pub struct PathSignals;

#[cfg(unix)]
use rustix::io::Errno;
#[cfg(unix)]
use rustix::process::{Pid, Signal, kill_process, test_kill_process};

#[cfg(unix)]
impl SignalPort for PathSignals {
    fn probe(&self, pid: u32) -> SignalOutcome {
        // kill(2) reads pid 0 and negative pids as process groups, never as the record's process
        let Some(target) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
            return SignalOutcome::Missing;
        };
        match test_kill_process(target) {
            Ok(()) => SignalOutcome::Exists,
            Err(Errno::SRCH) => SignalOutcome::Missing,
            Err(Errno::PERM) => SignalOutcome::Denied,
            // An error no probe expects still means betterlite cannot prove the process is gone
            Err(_other) => SignalOutcome::Exists,
        }
    }

    fn send(&self, pid: u32, signal: TermSignal) {
        let Some(target) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
            return;
        };
        let delivery = match signal {
            TermSignal::Term => Signal::TERM,
            TermSignal::Kill => Signal::KILL,
        };
        // A missing process is what a signal after the grace period races against
        let _ignored = kill_process(target, delivery);
    }
}

#[cfg(not(unix))]
impl SignalPort for PathSignals {
    fn probe(&self, _pid: u32) -> SignalOutcome {
        SignalOutcome::Missing
    }

    fn send(&self, _pid: u32, _signal: TermSignal) {}
}
