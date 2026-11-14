use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use betterlite_cli::agent_launch::{KeyProbe, Terminal};
use betterlite_cli::login::error::LoginError;
use betterlite_cli::token_records::record::CliTokenRecord;
use betterlite_cli::up::error::UpError;
use betterlite_cli::up::ports::{LoginAttempt, LoginLauncher, StopSource};
use betterlite_config::loader::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::fakes::{Probe, Snapshot, State, json_file};

pub(crate) struct FakeTerminal {
    pub(crate) interactive: bool,
}

impl Terminal for FakeTerminal {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn open_controlling(&self) -> io::Result<File> {
        Err(io::Error::other("no controlling terminal in the helper"))
    }

    fn attach_stdin(&self, _terminal: File) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct RecordingProbe {
    pub(crate) status: u16,
    pub(crate) state: Arc<State>,
}

impl KeyProbe for RecordingProbe {
    fn status(&self, url: &str, api_key: &SecretString, timeout: Duration) -> io::Result<u16> {
        let probe = Probe {
            url: url.to_owned(),
            timeout: timeout.as_secs(),
            authorization: format!("Bearer {}", api_key.expose_secret()),
        };
        *self
            .state
            .probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(probe);
        Ok(self.status)
    }
}

pub(crate) struct ScriptedLogin {
    pub(crate) token: Option<CliTokenRecord>,
    pub(crate) state: Arc<State>,
}

impl<H, E, W> LoginLauncher<H, E, W> for ScriptedLogin {
    fn start(&mut self, _attempt: LoginAttempt<'_, H, E, W>) -> Result<(), LoginError> {
        self.state
            .record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone_from(&self.token);
        self.state.logins.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(crate) struct ScriptedStop {
    pub(crate) state: Arc<State>,
    pub(crate) settings: PathBuf,
    pub(crate) backup: PathBuf,
}

impl StopSource for ScriptedStop {
    fn install(&self) -> Result<(), UpError> {
        let snapshot = Snapshot {
            settings: json_file(&self.settings),
            backup: json_file(&self.backup),
        };
        *self
            .state
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
        self.state.stopped.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn wait(&self) {}
}

pub(crate) struct FakeEnvironment {
    pub(crate) home: PathBuf,
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, _name: &str) -> Option<String> {
        None
    }

    fn system_home(&self) -> Option<PathBuf> {
        Some(self.home.clone())
    }
}
