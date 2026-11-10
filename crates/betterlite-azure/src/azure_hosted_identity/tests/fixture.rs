#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::Mutex;

use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_arc_identity::{ArcHost, ArcPlatform};
use crate::azure_client_secret::transport::{Clock, HttpMethod};
use crate::azure_hosted_identity::credential::{
    CloudShellCredential, HostedCredential, HostedIdentity,
};
use crate::azure_hosted_identity::environment::HostedEnvironment;
use crate::azure_hosted_identity::msal_reply::MsalKind;
use crate::azure_hosted_identity::protocol::{HostedProtocol, select_hosted_protocol};
use crate::azure_hosted_identity::request::{
    HostedIo, HostedRequest, HostedResponse, HostedTransport, TokenResult,
};

pub(super) const RESOURCE: &str = "https://cognitiveservices.azure.com";
pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const THUMBPRINT: &str = "00112233445566778899aabbccddeeff00112233";
pub(super) const OTHER_THUMBPRINT: &str = "ffeeddccbbaa99887766554433221100ffeeddcc";

pub(super) fn response(body: &str) -> HostedResponse {
    typed_response("application/json", body)
}

pub(super) fn typed_response(content_type: &str, body: &str) -> HostedResponse {
    HostedResponse {
        content_type: Some(content_type.to_owned()),
        body: SecretString::from(body.to_owned()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) method: HttpMethod,
    pub(super) url: String,
    pub(super) query: Vec<(String, String)>,
    pub(super) headers: Vec<(String, String)>,
    pub(super) form: Vec<(String, String)>,
}

impl Recorded {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn query_value(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(super) fn form_value(&self, name: &str) -> Option<&str> {
        self.form
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

type Acquired = (TokenResult, FixtureTransport);
type Replies = Vec<HostedResponse>;

pub(super) struct FixtureTransport {
    replies: Mutex<VecDeque<HostedResponse>>,
    requests: Mutex<Vec<Recorded>>,
}

impl FixtureTransport {
    pub(super) fn new(replies: Vec<HostedResponse>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

impl HostedTransport for FixtureTransport {
    fn send(&self, request: &HostedRequest) -> io::Result<HostedResponse> {
        self.requests
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .push(Recorded {
                method: request.method,
                url: request.url.clone(),
                query: request.query.clone(),
                headers: request
                    .headers
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), value.expose_secret().to_owned()))
                    .collect(),
                form: request.form.clone(),
            });
        self.replies
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .pop_front()
            .ok_or_else(|| io::Error::other("no reply queued"))
    }
}

pub(super) struct FixtureClock {
    values: Mutex<VecDeque<i64>>,
    fallback: i64,
}

impl FixtureClock {
    pub(super) fn fixed(now: i64) -> Self {
        Self {
            values: Mutex::new(VecDeque::new()),
            fallback: now,
        }
    }

    pub(super) fn scheduled(values: &[i64], fallback: i64) -> Self {
        Self {
            values: Mutex::new(values.iter().copied().collect()),
            fallback,
        }
    }
}

impl Clock for FixtureClock {
    fn now_unix_seconds(&self) -> i64 {
        self.values
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(self.fallback)
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct Fixture {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) client_id: Option<String>,
    pub(super) identity_config: BTreeMap<String, String>,
    pub(super) within_default_credential: bool,
}

impl Fixture {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn with_environment(mut self, pairs: &[(&str, &str)]) -> Self {
        for (name, value) in pairs {
            self.environment
                .insert((*name).to_owned(), (*value).to_owned());
        }
        self
    }

    pub(super) fn with_client_id(mut self, client_id: &str) -> Self {
        self.client_id = Some(client_id.to_owned());
        self
    }

    pub(super) fn with_identity_config(mut self, pairs: &[(&str, &str)]) -> Self {
        for (name, value) in pairs {
            self.identity_config
                .insert((*name).to_owned(), (*value).to_owned());
        }
        self
    }

    pub(super) fn within_default_credential(mut self) -> Self {
        self.within_default_credential = true;
        self
    }

    pub(super) fn hosted(&self) -> HostedCredential {
        HostedCredential {
            environment: self.environment(),
            identity: self.identity(),
        }
    }

    pub(super) fn cloud_shell(&self) -> CloudShellCredential {
        CloudShellCredential {
            environment: self.environment(),
            identity: self.identity(),
            within_default_credential: self.within_default_credential,
        }
    }

    fn environment(&self) -> HostedEnvironment {
        HostedEnvironment::from_environment(&self.environment)
    }

    fn identity(&self) -> HostedIdentity {
        HostedIdentity {
            client_id: self.client_id.clone(),
            identity_config: self.identity_config.clone(),
        }
    }

    pub(super) fn acquire(
        &self,
        protocol: MsalKind,
        replies: Vec<HostedResponse>,
        clock: &FixtureClock,
    ) -> Acquired {
        run(replies, clock, |io| {
            self.hosted().acquire(protocol, io, RESOURCE)
        })
    }

    pub(super) fn acquire_cloud_shell(&self, replies: Replies, clock: &FixtureClock) -> Acquired {
        run(replies, clock, |io| {
            self.cloud_shell().acquire(io, RESOURCE)
        })
    }
}

fn run<Call>(replies: Replies, clock: &FixtureClock, call: Call) -> Acquired
where
    Call: FnOnce(&HostedIo<'_>) -> TokenResult,
{
    let transport = FixtureTransport::new(replies);
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let outcome = call(&HostedIo {
        transport: &transport,
        clock,
        redactor: &redactor,
    });
    (outcome, transport)
}

#[derive(Debug, Clone)]
pub(super) struct ArcFixture {
    environment: BTreeMap<String, String>,
    platform: ArcPlatform,
    files: BTreeMap<String, String>,
}

impl ArcFixture {
    pub(super) fn new() -> Self {
        Self {
            environment: BTreeMap::new(),
            platform: ArcPlatform::Linux,
            files: BTreeMap::new(),
        }
    }

    pub(super) fn with_environment(mut self, pairs: &[(&str, &str)]) -> Self {
        for (name, value) in pairs {
            self.environment
                .insert((*name).to_owned(), (*value).to_owned());
        }
        self
    }

    pub(super) fn with_file(mut self, path: &str, contents: &str) -> Self {
        self.files.insert(path.to_owned(), contents.to_owned());
        self
    }

    pub(super) fn select(&self) -> HostedProtocol {
        select_hosted_protocol(
            &HostedEnvironment::from_environment(&self.environment),
            self,
        )
    }
}

impl ArcHost for ArcFixture {
    fn platform(&self) -> ArcPlatform {
        self.platform.clone()
    }

    fn environment(&self, name: &str) -> Option<String> {
        self.environment.get(name).cloned()
    }

    fn program_data(&self) -> String {
        "%ProgramData%".to_owned()
    }

    fn program_files(&self) -> String {
        "${ProgramFiles}".to_owned()
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
        NOW
    }
}

pub(super) fn only_request(transport: &FixtureTransport) -> Recorded {
    let mut requests = transport.recorded();
    assert_eq!(requests.len(), 1);
    requests.remove(0)
}

pub(super) fn assert_pairs(actual: &[(String, String)], expected: &[(&str, &str)]) {
    let observed: Vec<(&str, &str)> = actual
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    assert_eq!(observed, expected);
}
