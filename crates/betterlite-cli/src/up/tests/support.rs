use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use tempfile::TempDir;

use crate::agent_launch::{KeyProbe, Terminal};
use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::keyring::{KeyringUnusable, SecretErase, SecretRead, SecretVault, SecretWrite};
use crate::login::error::LoginError;
use crate::pkce_login::{Clock, Entropy, Http, HttpBody, HttpResponse, PkceFailure, WallClock};
use crate::sso_polling::polling::Sleeper;
use crate::sso_polling::terminal::Terminal as SelectionTerminal;
use crate::sso_polling::transport::PollTransport;
use crate::token_records::error::TokenRecordError;
use crate::token_records::record::CliTokenRecord;
use crate::up::error::UpError;
use crate::up::ports::{FlowLogin, LoginAttempt, LoginLauncher, StopSource, UpPorts};

pub(crate) const SERVER: &str = "http://gateway.test:4000";
const CLOCK_SECONDS: i64 = 2_000_000_000;

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

pub(crate) struct MemoryFiles {
    pub(crate) state: Arc<State>,
    pub(crate) path: PathBuf,
}

#[async_trait::async_trait]
impl CredentialFiles for MemoryFiles {
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
        Err(PkceFailure::Protocol("the test never sends".to_owned()))
    }

    fn post(
        &self,
        _url: &str,
        _body: HttpBody,
        _timeout_seconds: u64,
        _follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        Err(PkceFailure::Protocol("the test never sends".to_owned()))
    }
}

impl PollTransport for UnusedHttp {
    fn get(
        &self,
        _url: &str,
        _headers: &[(String, SecretString)],
        _timeout_seconds: u64,
    ) -> Result<HttpResponse, io::Error> {
        Err(io::Error::other("the test never polls"))
    }

    fn post(&self, _url: &str, _timeout_seconds: u64) -> Result<HttpResponse, io::Error> {
        Err(io::Error::other("the test never polls"))
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

pub(crate) struct FakeTerminal {
    pub(crate) interactive: bool,
}

impl Terminal for FakeTerminal {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn open_controlling(&self) -> io::Result<File> {
        Err(io::Error::other("no controlling terminal in the test"))
    }

    fn attach_stdin(&self, _terminal: File) -> io::Result<()> {
        Ok(())
    }
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
    pub(crate) real: bool,
}

impl<H, E, W> LoginLauncher<H, E, W> for ScriptedLogin
where
    H: Http + PollTransport + Send + Sync + 'static,
    E: Entropy,
    W: Clock + WallClock + Send + Sync + 'static,
{
    fn start(&mut self, attempt: LoginAttempt<'_, H, E, W>) -> Result<(), LoginError> {
        if self.real {
            return FlowLogin.start(attempt);
        }
        *self
            .state
            .record
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = self.token.clone();
        *self
            .state
            .logins
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
        Ok(())
    }
}

pub(crate) struct ScriptedStop {
    pub(crate) state: Arc<State>,
    pub(crate) settings: PathBuf,
    pub(crate) backup: PathBuf,
    #[cfg(unix)]
    pub(crate) real: Option<crate::up::signals::SignalStop>,
    pub(crate) installed: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl StopSource for ScriptedStop {
    fn install(&self) -> Result<(), UpError> {
        #[cfg(unix)]
        {
            if let Some(real) = &self.real {
                real.install()?;
            }
        }
        let snapshot = Snapshot {
            settings: json_file(&self.settings),
            backup: json_file(&self.backup),
        };
        *self
            .state
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(snapshot);
        self.state
            .stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(installed) = self
            .installed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _unused = installed.send(());
        }
        Ok(())
    }

    fn wait(&self) {
        #[cfg(unix)]
        {
            if let Some(real) = &self.real {
                real.wait();
            }
        }
    }
}

fn ignore_message(_message: &str) {}

pub(crate) fn json_file(path: &Path) -> Value {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
        Err(_unreadable) => Value::Null,
    }
}

pub(crate) struct World {
    pub(crate) context: CliContext,
    pub(crate) environment: FakeEnvironment,
    pub(crate) files: MemoryFiles,
    pub(crate) vault: DisabledVault,
    pub(crate) private_files: PrivateFiles,
    pub(crate) http: Arc<UnusedHttp>,
    pub(crate) clock: Arc<FixedClock>,
    pub(crate) entropy: ZeroEntropy,
    pub(crate) sleeper: NoSleeper,
    pub(crate) selection: SelectionTerminal,
    pub(crate) interrupt: crate::login::NeverInterrupted,
    pub(crate) token_path: PathBuf,
    pub(crate) binary_path: PathBuf,
    pub(crate) home: TempDir,
    pub(crate) probe: RecordingProbe,
    pub(crate) terminal: FakeTerminal,
    pub(crate) state: Arc<State>,
    pub(crate) login: ScriptedLogin,
    pub(crate) stop: ScriptedStop,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

impl World {
    pub(crate) fn new(interactive: bool) -> Self {
        let home = tempfile::tempdir().expect("a temporary home");
        let root = home.path().to_path_buf();
        let state = Arc::new(State::default());
        let token_path = root.join(".litellm").join("token.json");
        Self {
            context: CliContext {
                base_url: SERVER.to_owned(),
                base_url_explicit: false,
                api_key: None,
                api_key_from_token_file: false,
            },
            environment: FakeEnvironment { home: root.clone() },
            files: MemoryFiles {
                state: Arc::clone(&state),
                path: token_path.clone(),
            },
            vault: DisabledVault,
            private_files: PrivateFiles::default(),
            http: Arc::new(UnusedHttp),
            clock: Arc::new(FixedClock),
            entropy: ZeroEntropy,
            sleeper: NoSleeper,
            selection: SelectionTerminal::default(),
            interrupt: crate::login::NeverInterrupted,
            token_path,
            binary_path: PathBuf::from("/opt/betterlite"),
            home,
            probe: RecordingProbe {
                status: 200,
                state: Arc::clone(&state),
            },
            terminal: FakeTerminal { interactive },
            state: Arc::clone(&state),
            login: ScriptedLogin {
                token: None,
                state: Arc::clone(&state),
                real: false,
            },
            stop: ScriptedStop {
                state,
                settings: root.join(".claude").join("settings.json"),
                backup: root.join(".litellm").join("claude_settings_backup.json"),
                #[cfg(unix)]
                real: None,
                installed: Mutex::new(None),
            },
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    #[cfg(unix)]
    pub(crate) fn use_real_signals(&mut self) -> std::sync::mpsc::Receiver<()> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.stop.real = Some(crate::up::signals::SignalStop::default());
        *self
            .stop
            .installed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(sender);
        receiver
    }

    pub(crate) fn home_path(&self) -> &Path {
        self.home.path()
    }

    pub(crate) fn settings_path(&self) -> PathBuf {
        self.home.path().join(".claude").join("settings.json")
    }

    pub(crate) fn backup_path(&self) -> PathBuf {
        self.home
            .path()
            .join(".litellm")
            .join("claude_settings_backup.json")
    }

    pub(crate) fn seed(&self, path: &Path, text: &str) {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory).expect("the seed directory");
        }
        fs::write(path, text).expect("the seed file");
    }

    pub(crate) fn seed_token(&self, record: CliTokenRecord) {
        *self
            .state
            .record
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(record);
    }

    pub(crate) fn ports(&mut self) -> UpPorts<'_, UnusedHttp, ZeroEntropy, FixedClock> {
        UpPorts {
            context: &self.context,
            environment: &self.environment,
            files: &self.files,
            vault: &mut self.vault,
            private_files: &self.private_files,
            http: Arc::clone(&self.http),
            clock: Arc::clone(&self.clock),
            entropy: &mut self.entropy,
            sleeper: &self.sleeper,
            selection: &mut self.selection,
            browser: &ignore_message,
            interrupt: &self.interrupt,
            token_path: &self.token_path,
            binary_path: Some(&self.binary_path),
            home: Some(self.home.path()),
            expiration_hours: 24,
            warn: &ignore_message,
            terminal: Some(&self.terminal),
            login: &mut self.login,
            probe: &self.probe,
            signals: &self.stop,
            stdout: &mut self.stdout,
            stderr: &mut self.stderr,
        }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub(crate) fn login_count(&self) -> usize {
        *self
            .state
            .logins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn probe_record(&self) -> Option<Probe> {
        self.state
            .probe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn snapshot(&self) -> Option<Snapshot> {
        self.state
            .snapshot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn stopped(&self) -> bool {
        self.state.stopped.load(std::sync::atomic::Ordering::SeqCst)
    }
}

pub(crate) fn record(key: &str, timestamp: f64, expires_at: Option<f64>) -> CliTokenRecord {
    let mut record = CliTokenRecord::default();
    record.base_url = SERVER.to_owned();
    record.key = Some(SecretString::from(key.to_owned()));
    record.timestamp = timestamp;
    record.expires_at = expires_at;
    record
}

pub(crate) fn pkce_record(key: &str, expires_at: f64) -> CliTokenRecord {
    let mut record = record(key, 1_000.0, Some(expires_at));
    record.refresh_token = Some(SecretString::from("rt-old".to_owned()));
    record
}
