use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::autoroute::process::control::{Clock, SignalOutcome, SignalPort, TermSignal};
use crate::autoroute::process::error::ProcessError;
use crate::autoroute::process::launcher::{ExitCode, LaunchedProcess};
use crate::autoroute::process::liveliness::LivelinessProbe;
use crate::autoroute::process::runtime::GatewayRuntime;

pub(crate) struct FakeRuntime {
    pub found: Option<PathBuf>,
    pub runs: bool,
}

impl GatewayRuntime for FakeRuntime {
    fn find(&self) -> Option<PathBuf> {
        self.found.clone()
    }

    fn runs(&self, _executable: &Path) -> bool {
        self.runs
    }
}

pub(crate) struct FakeClock {
    base: Instant,
    elapsed: Cell<Duration>,
    sleeps: Cell<u32>,
}

impl Default for FakeClock {
    fn default() -> Self {
        Self {
            base: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
            sleeps: Cell::new(0),
        }
    }
}

impl FakeClock {
    pub(crate) fn sleeps(&self) -> u32 {
        self.sleeps.get()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.base
            .checked_add(self.elapsed.get())
            .unwrap_or(self.base)
    }

    fn sleep(&self, duration: Duration) {
        let next = self
            .elapsed
            .get()
            .checked_add(duration)
            .unwrap_or_else(|| self.elapsed.get());
        self.elapsed.set(next);
        self.sleeps.set(self.sleeps.get().saturating_add(1));
    }
}

pub(crate) struct FixedProbe(pub SignalOutcome);

impl SignalPort for FixedProbe {
    fn probe(&self, _pid: u32) -> SignalOutcome {
        self.0
    }

    fn send(&self, _pid: u32, _signal: TermSignal) {}
}

pub(crate) struct ScriptedSignals {
    pub alive: Mutex<bool>,
    pub exits_on_term: bool,
    pub sent: Mutex<Vec<TermSignal>>,
}

impl ScriptedSignals {
    pub(crate) fn sent(&self) -> Vec<TermSignal> {
        self.sent.lock().unwrap().clone()
    }

    pub(crate) fn alive(&self) -> bool {
        *self.alive.lock().unwrap()
    }
}

impl SignalPort for ScriptedSignals {
    fn probe(&self, _pid: u32) -> SignalOutcome {
        if self.alive() {
            SignalOutcome::Exists
        } else {
            SignalOutcome::Missing
        }
    }

    fn send(&self, _pid: u32, signal: TermSignal) {
        self.sent.lock().unwrap().push(signal);
        if signal == TermSignal::Kill || (self.exits_on_term && signal == TermSignal::Term) {
            *self.alive.lock().unwrap() = false;
        }
    }
}

pub(crate) struct FakeProcess {
    pub pid: u32,
    pub exit: Option<ExitCode>,
    pub checks: Cell<u32>,
}

impl LaunchedProcess for FakeProcess {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn has_exited(&mut self) -> Result<Option<ExitCode>, ProcessError> {
        self.checks.set(self.checks.get().saturating_add(1));
        Ok(self.exit)
    }
}

pub(crate) struct FakeLiveliness {
    pub statuses: Mutex<Vec<bool>>,
    pub answer_when_empty: bool,
    pub urls: Mutex<Vec<String>>,
}

impl FakeLiveliness {
    pub(crate) fn urls(&self) -> Vec<String> {
        self.urls.lock().unwrap().clone()
    }
}

impl LivelinessProbe for FakeLiveliness {
    fn is_live(&self, url: &str) -> bool {
        self.urls.lock().unwrap().push(url.to_owned());
        let mut statuses = self.statuses.lock().unwrap();
        if statuses.is_empty() {
            return self.answer_when_empty;
        }
        statuses.remove(0)
    }
}
