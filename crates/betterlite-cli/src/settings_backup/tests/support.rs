use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use betterlite_logging::private_json::files::PrivateFiles;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credential_lifecycle::files::ManagedTokenFile;
use crate::keyring::{
    Erase, KeyringUnusable, Read, SecretErase, SecretRead, SecretVault, SecretWrite, Write,
};
use crate::pkce_login::models::PkceFailure;
use crate::pkce_login::{Http, HttpBody, HttpResponse, WallClock};
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

pub(crate) fn token_file(directory: &Path) -> ManagedTokenFile {
    ManagedTokenFile {
        files: PrivateFiles::default(),
        path: directory.join("token.json"),
    }
}

pub(crate) fn seed(files: &ManagedTokenFile, stored: &CliTokenRecord) {
    let bytes = serde_json::to_vec(stored).expect("encodes the stored credential");
    fs::write(&files.path, bytes).expect("writes the stored credential");
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

pub(crate) struct FakeVault {
    pub(crate) blob: Option<SecretString>,
    pub(crate) read_failure: Option<KeyringUnusable>,
}

impl FakeVault {
    pub(crate) fn unreachable_read() -> Self {
        Self {
            blob: None,
            read_failure: Some(KeyringUnusable::KeyringUnreachable),
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
        self.blob = Some(blob.clone());
        Ok(Write::SecretStored)
    }

    fn erase(&mut self) -> SecretErase {
        self.blob = None;
        Ok(Erase::SecretErased)
    }
}

pub(crate) enum Reply {
    Answer(HttpResponse),
}

pub(crate) struct FakeHttp {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<String>>,
}

impl FakeHttp {
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(&self, url: &str, body: HttpBody) -> Result<HttpResponse, PkceFailure> {
        let _body = body;
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(Reply::Answer(response)) => Ok(response),
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

pub(crate) struct FixedWallClock(pub(crate) i64);

impl WallClock for FixedWallClock {
    fn now_seconds(&self) -> i64 {
        self.0
    }
}
