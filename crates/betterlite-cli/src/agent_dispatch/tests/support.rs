use std::fs::File;
use std::io::{self, Error as IoError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};
use console::Key;
use secrecy::SecretString;
use tempfile::TempDir;

use crate::agent_dispatch::error::AgentDispatchError;
use crate::agent_dispatch::key::resolve_api_key;
use crate::agent_dispatch::{AgentCommandArgs, AgentPorts, run_agent_command};
use crate::agent_launch::{EnvVars, KeyProbe, Platform, ProcessLauncher, Terminal, WindowsCommand};
use crate::agent_run::error::AgentRunError;
use crate::agent_run::messages::Messages;
use crate::agent_run::prepare::{PrepareRequest, Preparer, Preparers};
use crate::agent_run::resolver::BinaryResolver;
use crate::agent_run::sync::{ModelSync, SyncRequest};
use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::credentials::notice::MessageSink;
use crate::keyring::{
    Erase, KeyringUnusable, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write,
};
use crate::login::Interrupts;
use crate::login::ports::TerminalWriter;
use crate::opencode::ModelSyncResult;
use crate::pkce_login::models::PkceFailure;
use crate::pkce_login::{Clock, Entropy, Http, HttpBody, HttpResponse, WallClock};
use crate::sso_polling::polling::Sleeper;
use crate::sso_polling::terminal::SelectionInput;
use crate::sso_polling::transport::PollTransport;
use crate::token_records::error::TokenRecordError;
use crate::token_records::record::CliTokenRecord;

pub(crate) const SERVER: &str = "https://gateway.example";
pub(crate) const START: &str = "/sso/cli/start";
pub(crate) const POLL: &str = "/sso/cli/poll/";

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn stored_record(key: Option<SecretString>) -> CliTokenRecord {
    CliTokenRecord {
        base_url: SERVER.to_owned(),
        key,
        user_id: "user-1".to_owned(),
        user_email: "user@example.com".to_owned(),
        user_role: "cli".to_owned(),
        auth_header_name: "Authorization".to_owned(),
        jwt_token: secret(""),
        timestamp: 1_000.0_f64,
        expires_at: None,
        refresh_token: None,
        client_id: None,
        token_endpoint: None,
        revocation_endpoint: None,
        resource: None,
        team_id: None,
    }
}

pub(crate) fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn sso_routes() -> Vec<(&'static str, HttpResponse)> {
    vec![
        (
            START,
            response(
                200,
                r#"{"login_id":"login-1","poll_secret":"poll-secret","user_code":"code-1"}"#,
            ),
        ),
        (
            POLL,
            response(
                200,
                r#"{"status":"ready","key":"sk-sso","user_id":"user-9"}"#,
            ),
        ),
    ]
}

pub(crate) struct ScriptedTransport {
    routes: Vec<(&'static str, HttpResponse)>,
    requests: Mutex<Vec<String>>,
}

impl ScriptedTransport {
    pub(crate) fn new(routes: Vec<(&'static str, HttpResponse)>) -> Self {
        Self {
            routes,
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn respond(&self, url: &str) -> HttpResponse {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
        self.routes
            .iter()
            .find(|(fragment, _response)| url.contains(fragment))
            .map_or_else(|| response(404, ""), |(_fragment, answer)| answer.clone())
    }
}

impl Http for ScriptedTransport {
    fn get(&self, url: &str, _timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        Ok(self.respond(url))
    }

    fn post(
        &self,
        url: &str,
        _body: HttpBody,
        _timeout_seconds: u64,
        _follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        Ok(self.respond(url))
    }
}

impl PollTransport for ScriptedTransport {
    fn get(
        &self,
        url: &str,
        _headers: &[(String, SecretString)],
        _timeout_seconds: u64,
    ) -> Result<HttpResponse, io::Error> {
        Ok(self.respond(url))
    }

    fn post(&self, url: &str, _timeout_seconds: u64) -> Result<HttpResponse, io::Error> {
        Ok(self.respond(url))
    }
}

pub(crate) struct FakeVault {
    pub(crate) blob: Option<SecretString>,
    pub(crate) writes: Vec<SecretString>,
    pub(crate) failure: Option<KeyringUnusable>,
}

impl FakeVault {
    pub(crate) fn working() -> Self {
        Self {
            blob: None,
            writes: Vec::new(),
            failure: None,
        }
    }
}

impl SecretVault for FakeVault {
    fn read(&self) -> SecretRead {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        Ok(self
            .blob
            .as_ref()
            .map_or(Read::SecretMissing, |blob| Read::SecretFound(blob.clone())))
    }

    fn write(&mut self, blob: &SecretString) -> SecretWrite {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        self.writes.push(blob.clone());
        self.blob = Some(blob.clone());
        Ok(Write::SecretStored)
    }

    fn erase(&mut self) -> SecretErase {
        self.blob = None;
        Ok(Erase::SecretErased)
    }
}

#[derive(Default)]
struct FileState {
    record: Option<CliTokenRecord>,
    staged: Vec<CliTokenRecord>,
}

pub(crate) struct MemoryFiles {
    state: Mutex<FileState>,
    pub(crate) refuse: bool,
}

impl MemoryFiles {
    pub(crate) fn empty() -> Self {
        Self {
            state: Mutex::new(FileState::default()),
            refuse: false,
        }
    }

    pub(crate) fn holding(record: CliTokenRecord) -> Self {
        let files = Self::empty();
        files
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record = Some(record);
        files
    }

    pub(crate) fn stored(&self) -> Option<CliTokenRecord> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record
            .clone()
    }

    fn remember(&self, record: &CliTokenRecord) {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record = Some(record.clone());
    }

    fn refused() -> TokenRecordError {
        TokenRecordError::Stage {
            source: PrivateJsonError::Stage(
                PathBuf::from("token.json"),
                IoError::other("the fixture refuses the stage"),
            ),
        }
    }
}

#[async_trait]
impl CredentialFiles for MemoryFiles {
    fn read(&self) -> Option<CliTokenRecord> {
        self.stored()
    }

    async fn stage(&self, record: &CliTokenRecord) -> Result<StagedFile, TokenRecordError> {
        if self.refuse {
            return Err(Self::refused());
        }
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .staged
            .push(record.clone());
        Ok(StagedFile {
            path: PathBuf::from("token.json.staged"),
        })
    }

    async fn commit(&self, _staged: &StagedFile) -> bool {
        if self.refuse {
            return false;
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(committed) = state.staged.pop() else {
            return false;
        };
        state.record = Some(committed);
        true
    }

    async fn discard(&self, _staged: &StagedFile) {
        let _dropped = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .staged
            .pop();
    }

    async fn write(&self, record: &CliTokenRecord) -> Result<(), TokenRecordError> {
        if self.refuse {
            return Err(Self::refused());
        }
        self.remember(record);
        Ok(())
    }

    async fn overwrite_scrubbed(&self, record: &CliTokenRecord) -> bool {
        if self.refuse {
            return false;
        }
        self.remember(record);
        true
    }

    async fn remove(&self) -> Result<(), IoError> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record = None;
        Ok(())
    }
}

pub(crate) struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

impl WallClock for FixedClock {
    fn now_seconds(&self) -> i64 {
        1_700_000_000
    }
}

pub(crate) struct FakeEntropy;

impl Entropy for FakeEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), PkceFailure> {
        for byte in bytes {
            *byte = 1;
        }
        Ok(())
    }
}

pub(crate) struct NoSleep;

impl Sleeper for NoSleep {
    fn sleep(&self, _interval: Duration) {}
}

pub(crate) struct NoSelection;

impl SelectionInput for NoSelection {
    fn is_interactive(&self) -> bool {
        false
    }

    fn read_key(&mut self) -> Option<Key> {
        None
    }

    fn read_line(&mut self) -> Option<String> {
        None
    }
}

pub(crate) struct NoInterrupt;

impl Interrupts for NoInterrupt {
    fn interrupted(&self) -> bool {
        false
    }
}

pub(crate) struct FakeEnvironment {
    pub(crate) home: Option<PathBuf>,
}

impl FakeEnvironment {
    pub(crate) fn at(home: &Path) -> Self {
        Self {
            home: Some(home.to_path_buf()),
        }
    }
}

impl EnvironmentLookup for FakeEnvironment {
    fn lookup(&self, _name: &str) -> Option<String> {
        None
    }

    fn system_home(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

pub(crate) struct FakeTerminal {
    pub(crate) interactive: bool,
}

impl Terminal for FakeTerminal {
    fn is_interactive(&self) -> bool {
        self.interactive
    }

    fn open_controlling(&self) -> io::Result<File> {
        Err(IoError::other("no controlling terminal in the test"))
    }

    fn attach_stdin(&self, _terminal: File) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct ScriptedResolver {
    pub(crate) found: Option<String>,
}

impl BinaryResolver for ScriptedResolver {
    fn resolve(&self, _command: &str) -> Option<String> {
        self.found.clone()
    }
}

pub(crate) struct RecordingProbe {
    pub(crate) status: u16,
    pub(crate) calls: Mutex<Vec<String>>,
}

impl KeyProbe for RecordingProbe {
    fn status(&self, url: &str, _api_key: &SecretString, _timeout: Duration) -> io::Result<u16> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
        Ok(self.status)
    }
}

pub(crate) struct NoSync;

impl ModelSync for NoSync {
    fn sync(&self, _request: &SyncRequest<'_>) -> Result<ModelSyncResult, AgentRunError> {
        Ok(ModelSyncResult::Env(EnvVars::new()))
    }
}

pub(crate) struct NoPreparer;

static NO_PREPARER: NoPreparer = NoPreparer;

impl Preparer for NoPreparer {
    fn prepare(
        &self,
        _request: &PrepareRequest<'_>,
        _messages: &mut dyn Messages,
    ) -> Result<Vec<String>, AgentRunError> {
        Ok(Vec::new())
    }
}

pub(crate) struct RecordingMessages {
    pub(crate) notices: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

impl Messages for RecordingMessages {
    fn warn(&mut self, message: &str) -> io::Result<()> {
        self.warnings.push(message.to_owned());
        Ok(())
    }

    fn notice(&mut self, message: &str) -> io::Result<()> {
        self.notices.push(message.to_owned());
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct Handoff {
    pub(crate) path: String,
    pub(crate) args: Vec<String>,
    pub(crate) env: EnvVars,
}

pub(crate) struct RecordingLauncher {
    pub(crate) handoffs: Mutex<Vec<Handoff>>,
}

impl ProcessLauncher for RecordingLauncher {
    fn replace(&self, path: &str, args: &[String], env: &EnvVars) -> io::Result<i32> {
        self.handoffs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Handoff {
                path: path.to_owned(),
                args: args.to_vec(),
                env: env.clone(),
            });
        Ok(0)
    }

    fn spawn(&self, _command: &WindowsCommand, _env: &EnvVars) -> io::Result<i32> {
        Ok(0)
    }
}

pub(crate) struct AgentWorld {
    pub(crate) context: CliContext,
    pub(crate) environment: FakeEnvironment,
    pub(crate) files: MemoryFiles,
    pub(crate) vault: FakeVault,
    pub(crate) private_files: PrivateFiles,
    pub(crate) http: Arc<ScriptedTransport>,
    pub(crate) clock: Arc<FixedClock>,
    pub(crate) entropy: FakeEntropy,
    pub(crate) sleeper: NoSleep,
    pub(crate) selection: NoSelection,
    pub(crate) interrupt: NoInterrupt,
    pub(crate) token_path: PathBuf,
    pub(crate) home: TempDir,
    pub(crate) base_env: EnvVars,
    pub(crate) resolver: ScriptedResolver,
    pub(crate) probe: RecordingProbe,
    pub(crate) sync: NoSync,
    pub(crate) preparers: Preparers<'static>,
    pub(crate) launcher: RecordingLauncher,
    pub(crate) terminal: FakeTerminal,
    pub(crate) messages: RecordingMessages,
}

impl AgentWorld {
    fn bind<'world>(
        &'world mut self,
        browser: MessageSink<'world>,
        stdout: &'world mut TerminalWriter,
    ) -> AgentPorts<'world, ScriptedTransport, FakeEntropy, FixedClock> {
        AgentPorts {
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
            browser,
            interrupt: &self.interrupt,
            token_path: &self.token_path,
            binary_path: None,
            base_env: &self.base_env,
            home: Some(self.home.path()),
            resolver: &self.resolver,
            probe: &self.probe,
            sync: &self.sync,
            preparers: &self.preparers,
            launcher: &self.launcher,
            platform: Platform::Unix,
            terminal: Some(&self.terminal),
            messages: &mut self.messages,
            stdout,
        }
    }
}

fn context() -> CliContext {
    CliContext {
        base_url: SERVER.to_owned(),
        base_url_explicit: false,
        api_key: None,
        api_key_from_token_file: false,
    }
}

pub(crate) fn world(routes: Vec<(&'static str, HttpResponse)>) -> AgentWorld {
    let home = tempfile::tempdir().expect("a temporary home");
    let token_path = home.path().join(".litellm").join("token.json");
    fixtures(home, token_path, routes)
}

fn fixtures(
    home: TempDir,
    token_path: PathBuf,
    routes: Vec<(&'static str, HttpResponse)>,
) -> AgentWorld {
    let launcher = RecordingLauncher {
        handoffs: Mutex::new(Vec::new()),
    };
    let messages = RecordingMessages {
        notices: Vec::new(),
        warnings: Vec::new(),
    };
    AgentWorld {
        context: context(),
        environment: FakeEnvironment::at(home.path()),
        files: MemoryFiles::empty(),
        vault: FakeVault::working(),
        private_files: PrivateFiles::default(),
        http: Arc::new(ScriptedTransport::new(routes)),
        clock: Arc::new(FixedClock),
        entropy: FakeEntropy,
        sleeper: NoSleep,
        selection: NoSelection,
        interrupt: NoInterrupt,
        token_path,
        base_env: EnvVars::new(),
        resolver: ScriptedResolver {
            found: Some(String::from("/usr/bin/claude")),
        },
        probe: RecordingProbe {
            status: 200,
            calls: Mutex::new(Vec::new()),
        },
        sync: NoSync,
        preparers: Preparers {
            pi: &NO_PREPARER,
            codex: &NO_PREPARER,
        },
        launcher,
        terminal: FakeTerminal { interactive: false },
        messages,
        home,
    }
}

enum Action<'action> {
    Resolve,
    Launch {
        binary: &'action str,
        args: AgentCommandArgs,
    },
}

struct Outcome {
    key: Option<SecretString>,
    exit_code: i32,
}

fn capture(
    world: &mut AgentWorld,
    action: Action<'_>,
) -> (Result<Outcome, AgentDispatchError>, String) {
    let browser = |_url: &str| {};
    let mut stdout: Vec<u8> = Vec::new();
    let result = {
        let mut ports = world.bind(&browser, &mut stdout);
        match action {
            Action::Resolve => resolve_api_key(&mut ports).map(|key| Outcome {
                key: Some(key),
                exit_code: 0,
            }),
            Action::Launch { binary, args } => {
                run_agent_command(binary, args, &mut ports).map(|exit_code| Outcome {
                    key: None,
                    exit_code,
                })
            }
        }
    };
    let text = String::from_utf8(stdout).expect("the captured output is UTF-8");
    (result, text)
}

pub(crate) fn resolve(
    world: &mut AgentWorld,
) -> (Result<SecretString, AgentDispatchError>, String) {
    let (result, text) = capture(world, Action::Resolve);
    (
        result.map(|outcome| outcome.key.expect("resolve returns a key")),
        text,
    )
}

pub(crate) fn launch(
    world: &mut AgentWorld,
    binary: &str,
    args: AgentCommandArgs,
) -> (Result<i32, AgentDispatchError>, String) {
    let (result, text) = capture(world, Action::Launch { binary, args });
    (result.map(|outcome| outcome.exit_code), text)
}
