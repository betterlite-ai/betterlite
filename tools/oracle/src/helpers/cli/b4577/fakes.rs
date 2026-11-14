use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cli::credential_lifecycle::files::CredentialFiles;
use betterlite_cli::keyring::{KeyringUnusable, SecretErase, SecretRead, SecretVault, SecretWrite};
use betterlite_cli::pkce_login::models::PkceFailure;
use betterlite_cli::pkce_login::{Clock, Entropy, Http, HttpBody, HttpResponse, WallClock};
use betterlite_cli::sso_polling::polling::Sleeper;
use betterlite_cli::sso_polling::transport::PollTransport;
use betterlite_cli::token_records::error::TokenRecordError;
use betterlite_cli::token_records::record::CliTokenRecord;
use betterlite_logging::private_json::files::StagedFile;
use secrecy::SecretString;
use serde_json::Value;
use std::time::Duration;

pub(crate) const CLOCK_SECONDS: i64 = 2_000_000_000;

#[derive(Clone, serde::Serialize)]
pub(crate) struct Probe {
    pub(crate) url: String,
    pub(crate) timeout: u64,
    pub(crate) authorization: String,
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct Snapshot {
    pub(crate) settings: Value,
    pub(crate) backup: Value,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) record: Mutex<Option<CliTokenRecord>>,
    pub(crate) logins: Mutex<usize>,
    pub(crate) probe: Mutex<Option<Probe>>,
    pub(crate) snapshot: Mutex<Option<Snapshot>>,
    pub(crate) stopped: AtomicBool,
}

pub(crate) struct ScriptedFiles {
    pub(crate) state: Arc<State>,
    pub(crate) path: PathBuf,
}

#[async_trait::async_trait]
impl CredentialFiles for ScriptedFiles {
    fn read(&self) -> Option<CliTokenRecord> {
        self.state
            .record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    async fn stage(&self, _record: &CliTokenRecord) -> Result<StagedFile, TokenRecordError> {
        Ok(StagedFile {
            path: self.path.clone(),
        })
    }

    async fn commit(&self, _staged: &StagedFile) -> bool {
        true
    }

    async fn discard(&self, _staged: &StagedFile) {}

    async fn write(&self, _record: &CliTokenRecord) -> Result<(), TokenRecordError> {
        Ok(())
    }

    async fn overwrite_scrubbed(&self, _record: &CliTokenRecord) -> bool {
        true
    }

    async fn remove(&self) -> Result<(), io::Error> {
        Ok(())
    }
}

pub(crate) struct DisabledVault;

impl SecretVault for DisabledVault {
    fn read(&self) -> SecretRead {
        Err(KeyringUnusable::KeyringDisabled)
    }

    fn write(&mut self, _blob: &SecretString) -> SecretWrite {
        Err(KeyringUnusable::KeyringDisabled)
    }

    fn erase(&mut self) -> SecretErase {
        Err(KeyringUnusable::KeyringDisabled)
    }
}

pub(crate) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }
}

impl WallClock for FixedClock {
    fn now_seconds(&self) -> i64 {
        CLOCK_SECONDS
    }
}

pub(crate) struct UnusedHttp;

impl Http for UnusedHttp {
    fn get(&self, _url: &str, _timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        Err(PkceFailure::Protocol(
            "the helper never sends requests".to_owned(),
        ))
    }

    fn post(
        &self,
        _url: &str,
        _body: HttpBody,
        _timeout_seconds: u64,
        _follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        Err(PkceFailure::Protocol(
            "the helper never sends requests".to_owned(),
        ))
    }
}

impl PollTransport for UnusedHttp {
    fn get(
        &self,
        _url: &str,
        _headers: &[(String, SecretString)],
        _timeout_seconds: u64,
    ) -> Result<HttpResponse, io::Error> {
        Err(io::Error::other("the helper never polls"))
    }

    fn post(&self, _url: &str, _timeout_seconds: u64) -> Result<HttpResponse, io::Error> {
        Err(io::Error::other("the helper never polls"))
    }
}

pub(crate) struct ZeroEntropy;

impl Entropy for ZeroEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), PkceFailure> {
        bytes.fill(0);
        Ok(())
    }
}

pub(crate) struct NoSleeper;

impl Sleeper for NoSleeper {
    fn sleep(&self, _interval: Duration) {}
}

pub(crate) fn json_file(path: &Path) -> Value {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
        Err(_unreadable) => Value::Null,
    }
}
