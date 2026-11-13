use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::{Error as IoError, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::TempDir;
use url::Url;

use crate::credential_lifecycle::files::{CredentialFiles, ManagedTokenFile};
use crate::credential_lifecycle::outcome::SecretSave;
use crate::credentials::contracts::CliTokenData;
use crate::credentials::renewal::RenewalStore;
use crate::keyring::{
    Erase, KeyringUnusable, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write,
};
use crate::pkce_login::models::PkceFailure;
use crate::pkce_login::{Clock, Entropy, Http, HttpBody, HttpResponse, WallClock};
use crate::token_records::error::TokenRecordError;
use crate::token_records::file::read_token_file;
use crate::token_records::record::CliTokenRecord;

pub(crate) const SERVER: &str = "https://gateway.example";
pub(crate) const OTHER_SERVER: &str = "https://other.example";
pub(crate) const TOKEN_ENDPOINT: &str = "https://gateway.example/token";
pub(crate) const REVOCATION_ENDPOINT: &str = "https://gateway.example/revoke";
pub(crate) const RESOURCE: &str = "https://gateway.example/v1";

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn record() -> CliTokenRecord {
    CliTokenRecord {
        base_url: SERVER.to_owned(),
        key: Some(secret("sk-secret")),
        user_id: "user-1".to_owned(),
        user_email: "user@example.com".to_owned(),
        user_role: "cli".to_owned(),
        auth_header_name: "Authorization".to_owned(),
        jwt_token: secret(""),
        timestamp: 1_000.0_f64,
        expires_at: None,
        refresh_token: Some(secret("refresh-secret")),
        client_id: Some("client-1".to_owned()),
        token_endpoint: Some(TOKEN_ENDPOINT.to_owned()),
        revocation_endpoint: Some(REVOCATION_ENDPOINT.to_owned()),
        resource: Some(RESOURCE.to_owned()),
        team_id: Some("team-1".to_owned()),
    }
}

pub(crate) fn record_expiring_at(expires_at: f64) -> CliTokenRecord {
    let mut expiring = record();
    expiring.expires_at = Some(expires_at);
    expiring
}

pub(crate) fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn redirect(location: &str) -> HttpResponse {
    HttpResponse {
        status: 307,
        headers: vec![(String::from("Location"), location.to_owned())],
        body: Vec::new(),
    }
}

pub(crate) fn token_body(access: &str, refresh: &str, expires_in: i64) -> String {
    format!(
        r#"{{"access_token":"{access}","expires_in":{expires_in},"refresh_token":"{refresh}"}}"#
    )
}

pub(crate) fn token_body_with_identity(
    access: &str,
    refresh: &str,
    expires_in: i64,
    user_id: &str,
    team_id: &str,
) -> String {
    format!(
        r#"{{"access_token":"{access}","expires_in":{expires_in},"refresh_token":"{refresh}","user_id":"{user_id}","team_id":"{team_id}"}}"#
    )
}

pub(crate) fn contract_body(base_url: &str) -> String {
    format!(
        r#"{{"contract_version":1,"issuer":"{base_url}","authorization_endpoint":"{base_url}/authorize","token_endpoint":"{base_url}/token","registration_endpoint":"{base_url}/register","revocation_endpoint":"{base_url}/revoke","resource":"{base_url}/v1","code_challenge_methods_supported":["S256"]}}"#
    )
}

pub(crate) enum Reply {
    Response(HttpResponse),
    Transport,
}

#[derive(Clone)]
pub(crate) struct RecordedRequest {
    pub(crate) url: String,
    pub(crate) body: HttpBody,
    pub(crate) timeout: u64,
    pub(crate) follow_redirects: bool,
}

pub(crate) struct ScriptedHttp {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<RecordedRequest>>,
    pub(crate) events: Arc<Mutex<Vec<String>>>,
}

pub(crate) fn scripted_http(replies: Vec<Reply>) -> ScriptedHttp {
    ScriptedHttp {
        replies: Mutex::new(VecDeque::from(replies)),
        requests: Mutex::new(Vec::new()),
        events: Arc::new(Mutex::new(Vec::new())),
    }
}

impl ScriptedHttp {
    pub(crate) fn with_events(mut self, events: Arc<Mutex<Vec<String>>>) -> Self {
        self.events = events;
        self
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(
        &self,
        url: &str,
        body: HttpBody,
        timeout_seconds: u64,
        follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("http {url}"));
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: url.to_owned(),
                body,
                timeout: timeout_seconds,
                follow_redirects,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(Reply::Response(response)) => Ok(response),
            Some(Reply::Transport) => Err(PkceFailure::Protocol("the transport failed".to_owned())),
            None => Err(PkceFailure::Protocol("no scripted reply".to_owned())),
        }
    }
}

impl Http for ScriptedHttp {
    fn get(&self, url: &str, timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        self.answer(url, HttpBody::Json(Vec::new()), timeout_seconds, false)
    }

    fn post(
        &self,
        url: &str,
        body: HttpBody,
        timeout_seconds: u64,
        follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        self.answer(url, body, timeout_seconds, follow_redirects)
    }
}

pub(crate) fn form_fields(body: &HttpBody) -> Vec<(String, String)> {
    let HttpBody::Form(fields) = body else {
        panic!("expected a form body");
    };
    fields
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

pub(crate) fn form_value(fields: &[(String, String)], name: &str) -> String {
    fields
        .iter()
        .find(|(field, _value)| field == name)
        .map(|(_field, value)| value.clone())
        .expect("the form field")
}

pub(crate) struct FixedEntropy;

impl Entropy for FixedEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), PkceFailure> {
        for byte in bytes {
            *byte = 1;
        }
        Ok(())
    }
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub(crate) struct FixedWallClock(pub(crate) i64);

impl WallClock for FixedWallClock {
    fn now_seconds(&self) -> i64 {
        self.0
    }
}

#[derive(Default)]
pub(crate) struct FakeVault {
    pub(crate) blob: Option<SecretString>,
    pub(crate) failure: Option<KeyringUnusable>,
    pub(crate) writes: Vec<SecretString>,
    pub(crate) events: Arc<Mutex<Vec<String>>>,
}

impl FakeVault {
    pub(crate) fn working() -> Self {
        Self::default()
    }

    pub(crate) fn unavailable(failure: KeyringUnusable) -> Self {
        Self {
            blob: None,
            failure: Some(failure),
            writes: Vec::new(),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn clear_events(&self) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
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
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(String::from("vault-save"));
        Ok(Write::SecretStored)
    }

    fn erase(&mut self) -> SecretErase {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        self.blob = None;
        Ok(Erase::SecretErased)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    Commit,
    Write,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StageRefusal {
    #[default]
    Off,
    Always,
}

pub(crate) struct TestFiles {
    pub(crate) inner: ManagedTokenFile,
    pub(crate) stage: StageRefusal,
    pub(crate) refusals: Vec<Refusal>,
    pub(crate) _directory: TempDir,
}

pub(crate) fn test_files() -> TestFiles {
    let directory = tempfile::tempdir().expect("makes a token directory");
    let path = directory.path().join("token.json");
    TestFiles {
        inner: ManagedTokenFile {
            files: PrivateFiles::default(),
            path,
        },
        stage: StageRefusal::Off,
        refusals: Vec::new(),
        _directory: directory,
    }
}

impl TestFiles {
    pub(crate) fn path(&self) -> &Path {
        &self.inner.path
    }

    pub(crate) fn stored(&self) -> Option<CliTokenRecord> {
        read_token_file(self.path())
    }
}

#[derive(Clone, Copy)]
enum Refused {
    Stage,
    Write,
}

fn refusal_error(refused: Refused, path: &Path) -> TokenRecordError {
    match refused {
        Refused::Stage => TokenRecordError::Stage {
            source: PrivateJsonError::Stage(
                path.to_path_buf(),
                IoError::other("the fixture refuses the stage"),
            ),
        },
        Refused::Write => TokenRecordError::Write {
            source: PrivateJsonError::Commit(
                path.to_path_buf(),
                IoError::other("the fixture refuses the write"),
            ),
        },
    }
}

#[async_trait]
impl CredentialFiles for TestFiles {
    fn read(&self) -> Option<CliTokenRecord> {
        self.inner.read()
    }

    async fn stage(&self, record: &CliTokenRecord) -> Result<StagedFile, TokenRecordError> {
        if self.stage == StageRefusal::Always {
            return Err(refusal_error(Refused::Stage, self.path()));
        }
        self.inner.stage(record).await
    }

    async fn commit(&self, staged: &StagedFile) -> bool {
        if self.refusals.contains(&Refusal::Commit) {
            return false;
        }
        self.inner.commit(staged).await
    }

    async fn discard(&self, staged: &StagedFile) {
        self.inner.discard(staged).await;
    }

    async fn write(&self, record: &CliTokenRecord) -> Result<(), TokenRecordError> {
        if self.refusals.contains(&Refusal::Write) {
            return Err(refusal_error(Refused::Write, self.path()));
        }
        self.inner.write(record).await
    }

    async fn overwrite_scrubbed(&self, record: &CliTokenRecord) -> bool {
        self.inner.overwrite_scrubbed(record).await
    }

    async fn remove(&self) -> Result<(), IoError> {
        self.inner.remove().await
    }
}

#[derive(Default)]
pub(crate) struct ScriptedStore {
    pub(crate) saved: Vec<CliTokenData>,
    pub(crate) reloaded: Option<CliTokenData>,
}

pub(crate) fn scripted_store(reloaded: Option<CliTokenData>) -> ScriptedStore {
    ScriptedStore {
        saved: Vec::new(),
        reloaded,
    }
}

#[async_trait]
impl RenewalStore for ScriptedStore {
    async fn save(&mut self, record: CliTokenData) -> SecretSave {
        self.saved.push(record);
        SecretSave::SecretStored
    }

    async fn reload(&mut self) -> Option<CliTokenData> {
        self.reloaded.clone()
    }
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
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects to the loopback");
    write!(
        stream,
        "GET {}?{} HTTP/1.1\r\nHost: localhost\r\n\r\n",
        target.path(),
        query
    )
    .expect("writes the callback request");
}
