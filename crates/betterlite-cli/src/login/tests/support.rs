use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, Error as IoError, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::PrivateFiles;
use console::Key;
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;
use tokio::runtime::Builder;
use url::Url;

use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::{CredentialFiles, ManagedTokenFile};
use crate::credential_lifecycle::outcome::{CredentialNotRecorded, CredentialNotSaved, SecretSave};
use crate::credential_lifecycle::save::save_cli_token;
use crate::credentials::notice::MessageSink;
use crate::keyring::{
    Erase, KeyringUnusable, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write,
};
use crate::login::error::{FlowError, LoginError};
use crate::login::finish::{LoginFinished, finish_login};
use crate::login::ports::{FlowPorts, LoginPorts, TerminalWriter};
use crate::login::{Interrupts, LoginArgs, run_login};
use crate::pkce_login::models::PkceFailure;
use crate::pkce_login::{Clock, Entropy, Http, HttpBody, HttpResponse, WallClock};
use crate::sso_polling::polling::Sleeper;
use crate::sso_polling::terminal::SelectionInput;
use crate::sso_polling::transport::PollTransport;
use crate::token_records::error::TokenRecordError;
use crate::token_records::file::read_token_file;
use crate::token_records::record::CliTokenRecord;
use crate::token_records::secret::{CliTokenSecret, decode_secret};

pub(crate) const SERVER: &str = "https://gateway.example";
pub(crate) const TOKEN_ENDPOINT: &str = "https://gateway.example/token";
pub(crate) const REVOCATION_ENDPOINT: &str = "https://gateway.example/revoke";
pub(crate) const RESOURCE: &str = "https://gateway.example/v1";
pub(crate) const DISCOVERY: &str = "/.well-known/litellm-cli-auth";
pub(crate) const START: &str = "/sso/cli/start";
pub(crate) const POLL: &str = "/sso/cli/poll/";

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn record() -> CliTokenRecord {
    CliTokenRecord {
        base_url: SERVER.to_owned(),
        key: Some(secret("sk-previous")),
        user_id: "user-1".to_owned(),
        user_email: "user@example.com".to_owned(),
        user_role: "cli".to_owned(),
        auth_header_name: "Authorization".to_owned(),
        jwt_token: secret(""),
        timestamp: 1_000.0_f64,
        expires_at: None,
        refresh_token: Some(secret("refresh-previous")),
        client_id: Some("client-1".to_owned()),
        token_endpoint: Some(TOKEN_ENDPOINT.to_owned()),
        revocation_endpoint: Some(REVOCATION_ENDPOINT.to_owned()),
        resource: Some(RESOURCE.to_owned()),
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

pub(crate) fn contract_body() -> String {
    format!(
        r#"{{"contract_version":1,"issuer":"{SERVER}","authorization_endpoint":"{SERVER}/authorize","token_endpoint":"{SERVER}/token","registration_endpoint":"{SERVER}/register","revocation_endpoint":"{SERVER}/revoke","resource":"{SERVER}/v1","code_challenge_methods_supported":["S256"]}}"#
    )
}

pub(crate) fn token_body() -> String {
    String::from(
        r#"{"access_token":"sk-access","expires_in":3600,"refresh_token":"refresh-1","user_id":"user-7","team_id":"team-3"}"#,
    )
}

pub(crate) fn start_body(prefilled: bool) -> String {
    let extra = if prefilled {
        r#","verification_uri_complete":"https://idp.example/verify?code=code-1""#
    } else {
        ""
    };
    format!(r#"{{"login_id":"login-1","poll_secret":"poll-secret","user_code":"code-1"{extra}}}"#)
}

pub(crate) fn ready_body() -> String {
    String::from(r#"{"status":"ready","key":"sk-sso","user_id":"user-9"}"#)
}

pub(crate) fn pkce_routes() -> Vec<(&'static str, HttpResponse)> {
    vec![
        (DISCOVERY, response(200, &contract_body())),
        ("/register", response(201, r#"{"client_id":"client-1"}"#)),
        ("/token", response(200, &token_body())),
        ("/revoke", response(200, "{}")),
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
    pub(crate) failure: Option<KeyringUnusable>,
    pub(crate) writes: Vec<SecretString>,
}

impl FakeVault {
    pub(crate) fn working() -> Self {
        Self {
            blob: None,
            failure: None,
            writes: Vec::new(),
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

fn refused_stage() -> TokenRecordError {
    TokenRecordError::Stage {
        source: PrivateJsonError::Stage(
            PathBuf::from("token.json"),
            IoError::other("the fixture refuses the stage"),
        ),
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

pub(crate) struct FakeClock;

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub(crate) struct FakeWallClock(pub(crate) i64);

impl WallClock for FakeWallClock {
    fn now_seconds(&self) -> i64 {
        self.0
    }
}

pub(crate) struct FakeSleeper {
    pub(crate) slept: Mutex<Vec<Duration>>,
}

impl Sleeper for FakeSleeper {
    fn sleep(&self, interval: Duration) {
        self.slept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(interval);
    }
}

pub(crate) struct FakeSelection;

impl SelectionInput for FakeSelection {
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

pub(crate) struct FakeInterrupt(pub(crate) bool);

impl Interrupts for FakeInterrupt {
    fn interrupted(&self) -> bool {
        self.0
    }
}

pub(crate) struct FakeEnvironment {
    pub(crate) home: Option<PathBuf>,
    pub(crate) variables: BTreeMap<String, String>,
}

impl FakeEnvironment {
    pub(crate) fn at(home: &Path) -> Self {
        Self {
            home: Some(home.to_path_buf()),
            variables: BTreeMap::new(),
        }
    }

    pub(crate) fn homeless() -> Self {
        Self {
            home: None,
            variables: BTreeMap::new(),
        }
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

fn record_call(log: &Mutex<Vec<String>>, url: &str) {
    let mut entries = log.lock().unwrap_or_else(PoisonError::into_inner);
    entries.push(url.to_owned());
}

pub(crate) struct World {
    pub(crate) context: CliContext,
    pub(crate) environment: FakeEnvironment,
    pub(crate) files: Box<dyn CredentialFiles>,
    pub(crate) vault: FakeVault,
    pub(crate) private_files: PrivateFiles,
    pub(crate) http: Arc<ScriptedTransport>,
    pub(crate) clock: FakeClock,
    pub(crate) wall_clock: FakeWallClock,
    pub(crate) entropy: FakeEntropy,
    pub(crate) sleeper: FakeSleeper,
    pub(crate) selection: FakeSelection,
    pub(crate) interrupt: FakeInterrupt,
    pub(crate) token_path: PathBuf,
    pub(crate) home: TempDir,
    pub(crate) browser_calls: Arc<Mutex<Vec<String>>>,
    pub(crate) delivers_callback: bool,
}

impl World {
    fn bind<'world>(
        &'world mut self,
        browser: MessageSink<'world>,
        stdout: &'world mut TerminalWriter,
    ) -> LoginPorts<'world, ScriptedTransport, FakeClock, FakeEntropy, FakeWallClock> {
        LoginPorts {
            flow: FlowPorts {
                context: &self.context,
                environment: &self.environment,
                files: &*self.files,
                vault: &mut self.vault,
                private_files: &self.private_files,
                http: Arc::clone(&self.http),
                clock: &self.clock,
                wall_clock: &self.wall_clock,
                entropy: &mut self.entropy,
                sleeper: &self.sleeper,
                selection: &mut self.selection,
                browser,
                interrupt: &self.interrupt,
                token_path: &self.token_path,
                binary_path: None,
            },
            stdout,
        }
    }
}

pub(crate) fn world(routes: Vec<(&'static str, HttpResponse)>) -> World {
    let home = tempfile::tempdir().expect("a temporary home");
    let token_path = home.path().join(".litellm").join("token.json");
    let files: Box<dyn CredentialFiles> = Box::new(ManagedTokenFile {
        files: PrivateFiles::default(),
        path: token_path.clone(),
    });
    World {
        context: CliContext {
            base_url: SERVER.to_owned(),
            base_url_explicit: false,
            api_key: None,
            api_key_from_token_file: false,
        },
        environment: FakeEnvironment::at(home.path()),
        files,
        vault: FakeVault::working(),
        private_files: PrivateFiles::default(),
        http: Arc::new(ScriptedTransport::new(routes)),
        clock: FakeClock,
        wall_clock: FakeWallClock(1_700_000_000),
        entropy: FakeEntropy,
        sleeper: FakeSleeper {
            slept: Mutex::new(Vec::new()),
        },
        selection: FakeSelection,
        interrupt: FakeInterrupt(false),
        token_path,
        home,
        browser_calls: Arc::new(Mutex::new(Vec::new())),
        delivers_callback: false,
    }
}

pub(crate) fn run(world: &mut World, args: LoginArgs) -> (Result<(), LoginError>, String) {
    let calls = Arc::clone(&world.browser_calls);
    let delivers = world.delivers_callback;
    let browser = move |url: &str| {
        record_call(&calls, url);
        if delivers {
            deliver_callback(url, &[("code", "auth-code")]);
        }
    };
    let mut stdout: Vec<u8> = Vec::new();
    let result = {
        let mut ports = world.bind(&browser, &mut stdout);
        run_login(args, &mut ports)
    };
    (
        result,
        String::from_utf8(stdout).expect("the captured output is UTF-8"),
    )
}

pub(crate) fn finish(
    world: &mut World,
    finished: LoginFinished<'_>,
) -> (Result<(), FlowError>, String) {
    let browser = |_url: &str| {};
    let mut stdout: Vec<u8> = Vec::new();
    let result = {
        let ports = world.bind(&browser, &mut stdout);
        finish_login(finished, &ports.flow, &mut *ports.stdout)
    };
    (
        result,
        String::from_utf8(stdout).expect("the captured output is UTF-8"),
    )
}

pub(crate) fn credential_not_saved() -> SecretSave {
    SecretSave::CredentialNotSaved(CredentialNotSaved {
        source: refused_stage(),
    })
}

pub(crate) fn credential_not_recorded() -> SecretSave {
    SecretSave::CredentialNotRecorded(CredentialNotRecorded)
}

pub(crate) fn seed(
    files: &dyn CredentialFiles,
    vault: &mut (dyn SecretVault + Send + Sync),
    record: CliTokenRecord,
) {
    let runtime = Builder::new_current_thread().build().expect("a runtime");
    let outcome = runtime.block_on(save_cli_token(files, vault, record));
    assert!(matches!(outcome, SecretSave::SecretStored));
}

pub(crate) fn stored(world: &World) -> Option<CliTokenRecord> {
    read_token_file(&world.token_path)
}

pub(crate) fn stored_secret(world: &World) -> Option<CliTokenSecret> {
    let blob = world.vault.blob.as_ref()?;
    decode_secret(blob.expose_secret(), SERVER)
}

pub(crate) fn deliver_callback(url: &str, extra: &[(&str, &str)]) {
    let authorize = Url::parse(url).expect("the authorize URL");
    let state = authorize
        .query_pairs()
        .find(|(name, _value)| name == "state")
        .map(|(_name, value)| value.into_owned())
        .expect("the authorize state");
    let mut query = format!("state={state}");
    for (name, value) in extra {
        write!(query, "&{name}={value}").expect("writes the query");
    }
    let redirect = authorize
        .query_pairs()
        .find(|(name, _value)| name == "redirect_uri")
        .map(|(_name, value)| value.into_owned())
        .expect("the redirect URI");
    let target = Url::parse(&redirect).expect("the redirect URL");
    let port = target.port().expect("the loopback port");
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("the loopback");
    write!(
        stream,
        "GET {}?{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
        target.path(),
        query
    )
    .expect("writes the callback request");
}
