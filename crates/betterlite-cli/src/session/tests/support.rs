use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::StagedFile;
use secrecy::{ExposeSecret as _, SecretString};

use crate::cli_context::CliContext;
use crate::credential_lifecycle::files::CredentialFiles;
use crate::keyring::{
    Erase, KeyringUnusable, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write,
};
use crate::pkce_login::models::PkceFailure;
use crate::pkce_login::{Http, HttpBody, HttpResponse, WallClock};
use crate::token_records::error::TokenRecordError;
use crate::token_records::record::CliTokenRecord;

pub(crate) const SERVER: &str = "https://gateway.example";
pub(crate) const OTHER_SERVER: &str = "https://other.example";
pub(crate) const TOKEN_ENDPOINT: &str = "https://gateway.example/token";
pub(crate) const REVOCATION_ENDPOINT: &str = "https://gateway.example/revoke";
pub(crate) const RESOURCE: &str = "https://gateway.example/v1";

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub(crate) fn exposed(value: &SecretString) -> &str {
    value.expose_secret()
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
        team_id: None,
    }
}

pub(crate) fn context(base_url: &str) -> CliContext {
    CliContext {
        base_url: base_url.to_owned(),
        base_url_explicit: false,
        api_key: None,
        api_key_from_token_file: false,
    }
}

pub(crate) fn response(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(crate) fn token_response(access: &str, refresh: &str, expires_in: i64) -> HttpResponse {
    response(
        200,
        &format!(
            r#"{{"access_token":"{access}","expires_in":{expires_in},"refresh_token":"{refresh}"}}"#
        ),
    )
}

pub(crate) enum EraseResult {
    Erased,
    Stranded,
    Unavailable(KeyringUnusable),
}

pub(crate) struct FakeVault {
    pub(crate) blob: Option<SecretString>,
    pub(crate) read_failure: Option<KeyringUnusable>,
    pub(crate) write_failure: Option<KeyringUnusable>,
    pub(crate) erase_result: EraseResult,
    pub(crate) writes: Vec<SecretString>,
    pub(crate) erases: usize,
}

impl FakeVault {
    pub(crate) fn unreachable_read() -> Self {
        Self {
            blob: None,
            read_failure: Some(KeyringUnusable::KeyringUnreachable),
            write_failure: None,
            erase_result: EraseResult::Erased,
            writes: Vec::new(),
            erases: 0,
        }
    }
}

impl SecretVault for FakeVault {
    fn read(&self) -> SecretRead {
        if let Some(failure) = self.read_failure {
            return Err(failure);
        }
        Ok(self
            .blob
            .as_ref()
            .map_or(Read::SecretMissing, |blob| Read::SecretFound(blob.clone())))
    }

    fn write(&mut self, blob: &SecretString) -> SecretWrite {
        self.writes.push(blob.clone());
        if let Some(failure) = self.write_failure {
            return Err(failure);
        }
        self.blob = Some(blob.clone());
        Ok(Write::SecretStored)
    }

    fn erase(&mut self) -> SecretErase {
        self.erases = self.erases.saturating_add(1);
        match self.erase_result {
            EraseResult::Erased => {
                self.blob = None;
                Ok(Erase::SecretErased)
            }
            EraseResult::Stranded => Ok(Erase::SecretStranded),
            EraseResult::Unavailable(failure) => Err(failure),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    Stage,
    Commit,
    Remove,
    Scrub,
}

#[derive(Default)]
struct FileState {
    record: Option<CliTokenRecord>,
    staged: Vec<CliTokenRecord>,
    committed: Vec<CliTokenRecord>,
    removals: usize,
}

pub(crate) struct FakeFiles {
    state: Mutex<FileState>,
    pub(crate) refusals: Vec<Refusal>,
}

impl FakeFiles {
    pub(crate) fn holding(record: CliTokenRecord) -> Self {
        let files = Self::empty();
        files.state().record = Some(record);
        files
    }

    pub(crate) fn empty() -> Self {
        Self {
            state: Mutex::new(FileState::default()),
            refusals: Vec::new(),
        }
    }

    fn state(&self) -> MutexGuard<'_, FileState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn refuses(&self, refusal: Refusal) -> bool {
        self.refusals.contains(&refusal)
    }

    pub(crate) fn removals(&self) -> usize {
        self.state().removals
    }

    pub(crate) fn committed(&self) -> Vec<CliTokenRecord> {
        self.state().committed.clone()
    }
}

fn refused() -> PrivateJsonError {
    PrivateJsonError::Stage(
        PathBuf::from("token.json"),
        io::Error::other("the fixture refuses the stage"),
    )
}

#[async_trait]
impl CredentialFiles for FakeFiles {
    fn read(&self) -> Option<CliTokenRecord> {
        self.state().record.clone()
    }

    async fn stage(&self, record: &CliTokenRecord) -> Result<StagedFile, TokenRecordError> {
        if self.refuses(Refusal::Stage) {
            return Err(TokenRecordError::Stage { source: refused() });
        }
        self.state().staged.push(record.clone());
        Ok(StagedFile {
            path: PathBuf::from("token.json.staged"),
        })
    }

    async fn commit(&self, _staged: &StagedFile) -> bool {
        if self.refuses(Refusal::Commit) {
            return false;
        }
        let mut state = self.state();
        let Some(committed) = state.staged.pop() else {
            return false;
        };
        state.record = Some(committed.clone());
        state.committed.push(committed);
        true
    }

    async fn discard(&self, _staged: &StagedFile) {
        let _dropped = self.state().staged.pop();
    }

    async fn write(&self, record: &CliTokenRecord) -> Result<(), TokenRecordError> {
        self.state().record = Some(record.clone());
        Ok(())
    }

    async fn overwrite_scrubbed(&self, record: &CliTokenRecord) -> bool {
        if self.refuses(Refusal::Scrub) {
            return false;
        }
        self.state().record = Some(record.clone());
        true
    }

    async fn remove(&self) -> Result<(), io::Error> {
        {
            let mut state = self.state();
            state.removals = state.removals.saturating_add(1);
            if !self.refuses(Refusal::Remove) {
                state.record = None;
            }
        }
        if self.refuses(Refusal::Remove) {
            return Err(io::Error::other("the fixture refuses the removal"));
        }
        Ok(())
    }
}

pub(crate) enum Reply {
    Answer(HttpResponse),
    Transport,
}

#[derive(Clone)]
pub(crate) struct RecordedRequest {
    pub(crate) url: String,
    pub(crate) body: HttpBody,
}

pub(crate) struct FakeHttp {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<RecordedRequest>>,
}

impl FakeHttp {
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(&self, url: &str, body: HttpBody) -> Result<HttpResponse, PkceFailure> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedRequest {
                url: url.to_owned(),
                body,
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(Reply::Answer(response)) => Ok(response),
            Some(Reply::Transport) => Err(PkceFailure::Protocol("the transport failed".to_owned())),
            None => Err(PkceFailure::Protocol("no scripted reply".to_owned())),
        }
    }
}

impl Http for FakeHttp {
    fn get(&self, url: &str, _timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        self.answer(url, HttpBody::Json(Vec::new()))
    }

    fn post(
        &self,
        url: &str,
        body: HttpBody,
        _timeout_seconds: u64,
        _follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        self.answer(url, body)
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

pub(crate) struct FixedWallClock(pub(crate) i64);

impl WallClock for FixedWallClock {
    fn now_seconds(&self) -> i64 {
        self.0
    }
}

pub(crate) struct FakeEnvironment {
    pub(crate) home: Option<PathBuf>,
    pub(crate) variables: BTreeMap<String, String>,
}

impl FakeEnvironment {
    pub(crate) fn at(home: &Path, variables: &[(&str, &str)]) -> Self {
        Self {
            home: Some(home.to_path_buf()),
            variables: variables
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
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
