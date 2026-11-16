use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use betterlite_config::loader::EnvironmentLookup;
use secrecy::SecretString;

use crate::agent_launch::EnvVars;
use crate::cli::session::{CliClock, Session, SessionParts};
use crate::cli::streams::{Channel, Streams};
use crate::cli::{Cli, run_with};
use crate::cli_context::CliContext;
use crate::commands::Client;
use crate::keyring::{
    Erase, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write as VaultWrite,
};

#[derive(Default)]
pub(crate) struct FakeEnvironment {
    pub(crate) variables: BTreeMap<String, String>,
    pub(crate) home: Option<PathBuf>,
}

impl FakeEnvironment {
    pub(crate) fn with_home(home: PathBuf) -> Self {
        Self {
            variables: BTreeMap::new(),
            home: Some(home),
        }
    }

    pub(crate) fn with_variable(mut self, name: &str, value: &str) -> Self {
        let _inserted = self.variables.insert(name.to_owned(), value.to_owned());
        self
    }
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, name: &str) -> Option<String> {
        self.variables.get(name).cloned()
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

#[derive(Default)]
pub(crate) struct FakeVault;

impl SecretVault for FakeVault {
    fn read(&self) -> SecretRead {
        Ok(Read::SecretMissing)
    }

    fn write(&mut self, _blob: &SecretString) -> SecretWrite {
        Ok(VaultWrite::SecretStored)
    }

    fn erase(&mut self) -> SecretErase {
        Ok(Erase::SecretErased)
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn text(&self) -> String {
        let bytes = self.0.lock().map(|bytes| bytes.clone()).unwrap_or_default();
        String::from_utf8(bytes).unwrap_or_default()
    }
}

pub(crate) fn channel(sink: Box<dyn Write + Send>) -> Channel {
    Channel(Arc::new(Mutex::new(sink)))
}

impl Write for Captured {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self.0.lock() {
            Ok(mut bytes) => bytes.extend_from_slice(buffer),
            Err(_poisoned) => return Err(io::Error::other("the capture lock was poisoned")),
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct Harness {
    pub(crate) environment: FakeEnvironment,
    pub(crate) vault: FakeVault,
    pub(crate) streams: Streams,
    pub(crate) out: Captured,
    pub(crate) err: Captured,
}

impl Harness {
    pub(crate) fn new(environment: FakeEnvironment) -> Self {
        let out = Captured::default();
        let err = Captured::default();
        Self {
            environment,
            vault: FakeVault,
            streams: Streams::new(Box::new(out.clone()), Box::new(err.clone())),
            out,
            err,
        }
    }

    pub(crate) fn run(&mut self, arguments: &[&str]) -> u8 {
        let arguments: Vec<String> = arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect();
        let cli = Cli {
            environment: &self.environment,
            vault: &mut self.vault,
            streams: self.streams.clone(),
            base_env: EnvVars::new(),
            stdin_is_terminal: false,
        };
        run_with(&arguments, cli)
    }

    pub(crate) fn stdout(&self) -> String {
        self.out.text()
    }

    pub(crate) fn stderr(&self) -> String {
        self.err.text()
    }
}

pub(crate) fn session<'session>(
    environment: &'session FakeEnvironment,
    vault: &'session mut FakeVault,
    base_url: &str,
    out: &Captured,
    err: &Captured,
) -> Session<'session> {
    let context = CliContext {
        base_url: base_url.to_owned(),
        base_url_explicit: false,
        api_key: None,
        api_key_from_token_file: false,
    };
    let client = Arc::new(Client::new(&context, environment).unwrap());
    let streams = Streams::new(Box::new(out.clone()), Box::new(err.clone()));
    Session::new(SessionParts {
        context: &context,
        environment,
        vault,
        streams,
        client,
        clock: Arc::new(CliClock),
        base_env: EnvVars::new(),
        stdin_is_terminal: false,
    })
    .unwrap()
}
