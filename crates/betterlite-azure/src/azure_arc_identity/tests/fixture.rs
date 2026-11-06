#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::Mutex;

use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_arc_identity::error::ArcIdentityError;
use crate::azure_arc_identity::{
    ArcAccessToken, ArcHost, ArcIdentity, ArcPlatform, ArcRequest, ArcResponse, ArcTransport,
    acquire_arc_token,
};

pub(super) const ENDPOINT: &str = "http://localhost:40342/metadata/identity/oauth2/token";
pub(super) const RESOURCE: &str = "https://cognitiveservices.azure.com";

pub(super) struct FixtureHost {
    pub(super) platform: ArcPlatform,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) program_data: String,
    pub(super) program_files: String,
    pub(super) files: BTreeMap<String, String>,
    pub(super) now: i64,
}

fn owned_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

impl FixtureHost {
    pub(super) fn linux(files: &[(&str, &str)]) -> Self {
        Self {
            platform: ArcPlatform::Linux,
            environment: BTreeMap::new(),
            program_data: "%ProgramData%".to_owned(),
            program_files: "%ProgramFiles%".to_owned(),
            files: owned_pairs(files).into_iter().collect(),
            now: 1_700_000_000,
        }
    }
}

impl ArcHost for FixtureHost {
    fn platform(&self) -> ArcPlatform {
        self.platform.clone()
    }

    fn environment(&self, name: &str) -> Option<String> {
        self.environment.get(name).cloned()
    }

    fn program_data(&self) -> String {
        self.program_data.clone()
    }

    fn program_files(&self) -> String {
        self.program_files.clone()
    }

    fn file_size(&self, path: &str) -> io::Result<u64> {
        let text = self
            .files
            .get(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))?;
        u64::try_from(text.len()).map_err(io::Error::other)
    }

    fn read_text(&self, path: &str) -> io::Result<String> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }

    fn now_unix_seconds(&self) -> i64 {
        self.now
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) query: Vec<(String, String)>,
    pub(super) headers: Vec<(String, String)>,
}

impl Recorded {
    pub(super) fn challenge() -> Self {
        Self {
            url: ENDPOINT.to_owned(),
            query: owned_pairs(&[("api-version", "2020-06-01"), ("resource", RESOURCE)]),
            headers: owned_pairs(&[("Metadata", "true")]),
        }
    }

    pub(super) fn authenticated(key_text: &str) -> Self {
        let mut request = Self::challenge();
        request
            .headers
            .push(("Authorization".to_owned(), format!("Basic {key_text}")));
        request
    }

    pub(super) fn authorization(&self) -> Option<String> {
        self.headers
            .iter()
            .find(|(name, _)| name == "Authorization")
            .map(|(_, value)| value.clone())
    }
}

pub(super) struct FixtureTransport {
    replies: Mutex<VecDeque<ArcResponse>>,
    requests: Mutex<Vec<Recorded>>,
}

impl FixtureTransport {
    pub(super) fn new(replies: Vec<ArcResponse>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }

    pub(super) fn challenge_then(challenge: &str, body: &str) -> Self {
        Self::new(vec![
            response(401, &[("WWW-Authenticate", challenge)], ""),
            response(200, &[], body),
        ])
    }
}

impl ArcTransport for FixtureTransport {
    fn get(&self, request: &ArcRequest) -> io::Result<ArcResponse> {
        self.requests
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .push(Recorded {
                url: request.url.clone(),
                query: request
                    .query
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), value.clone()))
                    .collect(),
                headers: request
                    .headers
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), value.expose_secret().to_owned()))
                    .collect(),
            });
        self.replies
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .pop_front()
            .ok_or_else(|| io::Error::other("no reply queued"))
    }
}

pub(super) fn response(status: u16, headers: &[(&str, &str)], body: &str) -> ArcResponse {
    ArcResponse {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: SecretString::from(body.to_owned()),
    }
}

pub(super) fn run(
    host: &FixtureHost,
    challenge: &str,
    body: &str,
) -> (Result<ArcAccessToken, ArcIdentityError>, FixtureTransport) {
    let transport = FixtureTransport::challenge_then(challenge, body);
    let result = acquire_arc_token(
        host,
        &transport,
        ENDPOINT,
        RESOURCE,
        ArcIdentity::SystemAssigned,
    );
    (result, transport)
}
