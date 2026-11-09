use std::convert::Infallible;
use std::io;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite::diagnostic_commands::ProcessLauncher;
use betterlite::startup_input::PortProber;
use betterlite_http::health_probe::{HealthTransport, TransportError};

pub(super) struct CannedTransport {
    pub(super) body: String,
    pub(super) url: Mutex<Option<String>>,
}

impl HealthTransport for CannedTransport {
    fn get(&self, url: &str) -> Result<String, TransportError> {
        *self.url.lock().unwrap_or_else(PoisonError::into_inner) = Some(url.to_owned());
        Ok(self.body.clone())
    }
}

#[derive(Default)]
pub(super) struct RecordingLauncher {
    pub(super) commands: Mutex<Vec<Vec<String>>>,
}

impl ProcessLauncher for RecordingLauncher {
    fn spawn_silent(&self, program: &str, args: &[&str]) -> Result<(), io::Error> {
        let mut command = Vec::with_capacity(args.len().saturating_add(1));
        command.push(program.to_owned());
        command.extend(args.iter().map(|arg| (*arg).to_owned()));
        self.commands
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(command);
        Ok(())
    }
}

pub(super) struct RecordingProber {
    pub(super) busy: bool,
    pub(super) asked: Mutex<Vec<u16>>,
}

impl PortProber for RecordingProber {
    fn in_use(&self, port: u16) -> bool {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(port);
        self.busy
    }
}

pub(super) struct RecordingRng {
    pub(super) sample: u32,
    pub(super) draws: AtomicUsize,
}

impl rand::TryRng for RecordingRng {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        let _previous = self.draws.fetch_add(1, Ordering::Relaxed);
        Ok(self.sample)
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        self.try_next_u32().map(u64::from)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        dst.fill(0);
        Ok(())
    }
}
