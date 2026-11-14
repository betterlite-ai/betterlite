use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::azure_oidc_exchange::Settings;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_arc_identity::{
    ArcHost, ArcPlatform, ArcTransport, MetadataRequest, MetadataResponse,
};
use crate::azure_certificate::assertion::IdSource;
use crate::azure_client_secret::transport::Clock;
use crate::azure_hosted_identity::request::{HostedRequest, HostedResponse, HostedTransport};
use crate::oidc_exchange::AzureOidcExchange;
use crate::oidc_exchange::transport::{Clock as OidcClock, HttpReply, Transport as OidcTransport};

use super::super::environment::ManagedIdentityEnvironment;
use super::super::error::ManagedIdentityError;
use super::super::imds::ImdsTransport;
use super::super::imds::ProbePolicy;
use super::super::runtime::{ImdsIo, ManagedIdentityIo, Scheduler, TokenFiles, WorkloadIo};
use super::super::{ManagedIdentityCredential, ManagedIdentityToken};

pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const REQUEST_ID: &str = "00000000-0000-4000-8000-000000000001";
pub(super) const APP_SERVICE: &str = "http://app.test/token";

#[derive(Clone)]
pub(super) enum Reply {
    Response {
        status: u16,
        headers: Vec<(&'static str, &'static str)>,
        body: String,
    },
    Failure,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self::Response {
            status: 200,
            headers: vec![("Content-Type", "application/json")],
            body: body.to_owned(),
        }
    }

    pub(super) fn json(status: u16, body: &str) -> Self {
        Self::Response {
            status,
            headers: vec![("Content-Type", "application/json")],
            body: body.to_owned(),
        }
    }

    pub(super) fn text(status: u16, body: &str) -> Self {
        Self::Response {
            status,
            headers: vec![("Content-Type", "text/plain")],
            body: body.to_owned(),
        }
    }

    pub(super) fn bare(status: u16, body: &str) -> Self {
        Self::Response {
            status,
            headers: Vec::new(),
            body: body.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RecordedRequest {
    pub(super) method: &'static str,
    pub(super) url: String,
    pub(super) query: BTreeMap<String, String>,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) timeout_seconds: u64,
}

impl RecordedRequest {
    pub(super) fn query(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(String::as_str)
    }

    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

struct ReplyParts {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl ReplyParts {
    fn imds(self) -> MetadataResponse {
        MetadataResponse {
            status: self.status,
            headers: self.headers,
            body: SecretString::from(self.body),
        }
    }

    fn arc(self) -> MetadataResponse {
        MetadataResponse {
            status: self.status,
            headers: self.headers,
            body: SecretString::from(self.body),
        }
    }
}

pub(super) struct Script<T> {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<T>>,
}

impl<T> Default for Script<T> {
    fn default() -> Self {
        Self {
            replies: Mutex::default(),
            requests: Mutex::default(),
        }
    }
}

impl<T> Script<T> {
    pub(super) fn push(&self, reply: Reply) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(reply);
    }

    pub(super) fn push_all(&self, replies: Vec<Reply>) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(replies);
    }

    pub(super) fn recorded(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn exchange(&self, request: T) -> io::Result<ReplyParts> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| io::Error::other("no reply queued"))?;
        match reply {
            Reply::Failure => Err(io::Error::other("transport down")),
            Reply::Response {
                status,
                headers,
                body,
            } => {
                let owned = headers
                    .into_iter()
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
                    .collect();
                Ok(ReplyParts {
                    status,
                    headers: owned,
                    body,
                })
            }
        }
    }
}

pub(super) type RecordingImds = Script<RecordedRequest>;
pub(super) type RecordingHosted = Script<RecordedHosted>;
pub(super) type RecordingArc = Script<MetadataRequest>;

impl From<&MetadataRequest> for RecordedRequest {
    fn from(request: &MetadataRequest) -> Self {
        Self {
            method: "GET",
            url: request.url.clone(),
            query: BTreeMap::from_iter(request.query.iter().cloned()),
            headers: request
                .headers
                .iter()
                .map(|(name, value)| (name.to_lowercase(), value.expose_secret().to_owned()))
                .collect(),
            timeout_seconds: request.timeout.as_secs(),
        }
    }
}

impl ImdsTransport for RecordingImds {
    fn get(&self, request: &MetadataRequest) -> io::Result<MetadataResponse> {
        Ok(self.exchange(RecordedRequest::from(request))?.imds())
    }
}

#[derive(Default)]
pub(super) struct RecordingScheduler {
    sleeps: Mutex<Vec<Duration>>,
}

impl RecordingScheduler {
    pub(super) fn recorded(&self) -> Vec<Duration> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Scheduler for RecordingScheduler {
    fn sleep(&self, delay: Duration) {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay);
    }
}

#[derive(Default)]
pub(super) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> String {
        REQUEST_ID.to_owned()
    }
}

pub(super) struct FixedClock {
    now: Mutex<i64>,
}

impl FixedClock {
    pub(super) fn fixed(now: i64) -> Self {
        Self {
            now: Mutex::new(now),
        }
    }
}

impl Clock for FixedClock {
    fn now_unix_seconds(&self) -> i64 {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
pub(super) struct FixtureFiles {
    files: Mutex<BTreeMap<String, String>>,
}

impl FixtureFiles {
    pub(super) fn insert(&self, path: &str, content: &str) {
        self.files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_owned(), content.to_owned());
    }
}

impl TokenFiles for FixtureFiles {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        let files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        files
            .get(path)
            .map(|content| content.as_bytes().to_vec())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }
}

#[derive(Debug, Clone)]
pub(super) struct RecordedHosted {
    pub(super) url: String,
    pub(super) headers: Vec<(&'static str, SecretString)>,
}

impl HostedTransport for RecordingHosted {
    fn send(&self, request: &HostedRequest) -> io::Result<HostedResponse> {
        let parts = self.exchange(RecordedHosted {
            url: request.url.clone(),
            headers: request.headers.clone(),
        })?;
        let content_type = parts
            .headers
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value);
        Ok(HostedResponse {
            content_type,
            body: SecretString::from(parts.body),
        })
    }
}

impl ArcTransport for RecordingArc {
    fn get(&self, request: &MetadataRequest) -> io::Result<MetadataResponse> {
        Ok(self.exchange(request.clone())?.arc())
    }
}

struct RecordedOidc {
    url: String,
    form: Vec<(String, SecretString)>,
}

#[derive(Default)]
pub(super) struct RecordingOidc {
    replies: Mutex<VecDeque<(u16, String)>>,
    requests: Mutex<Vec<RecordedOidc>>,
}

impl RecordingOidc {
    pub(super) fn push(&self, status: u16, body: &str) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back((status, body.to_owned()));
    }

    pub(super) fn recorded(&self) -> Vec<BTreeMap<String, String>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|request| {
                let mut recorded = BTreeMap::from([("url".to_owned(), request.url.clone())]);
                recorded.extend(
                    request
                        .form
                        .iter()
                        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned())),
                );
                recorded
            })
            .collect()
    }
}

impl OidcTransport for RecordingOidc {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<HttpReply> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedOidc {
                url: url.to_owned(),
                form: form.to_vec(),
            });
        let (status, body) = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| io::Error::other("no oidc reply queued"))?;
        Ok(HttpReply {
            status,
            body: SecretString::from(body),
        })
    }
}

pub(super) struct FixedOidcClock;

impl OidcClock for FixedOidcClock {
    fn now_unix_seconds(&self) -> f64 {
        1_700_000_000.0_f64
    }
}

pub(super) struct FixtureHost {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) files: BTreeMap<String, String>,
    pub(super) platform: String,
}

impl ArcHost for FixtureHost {
    fn platform(&self) -> ArcPlatform {
        match self.platform.as_str() {
            "win32" => ArcPlatform::Windows,
            "" | "linux" => ArcPlatform::Linux,
            other => ArcPlatform::Other(other.to_owned()),
        }
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
        u64::try_from(self.read_text(path)?.len()).map_err(io::Error::other)
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

pub(super) struct Fixture {
    pub(super) environment: ManagedIdentityEnvironment,
    pub(super) host: FixtureHost,
    pub(super) imds: RecordingImds,
    pub(super) scheduler: RecordingScheduler,
    pub(super) ids: FixedIds,
    pub(super) clock: FixedClock,
    pub(super) hosted: RecordingHosted,
    pub(super) arc: RecordingArc,
    pub(super) files: FixtureFiles,
    pub(super) oidc: Arc<RecordingOidc>,
    redactor: Redactor,
    exchange: AzureOidcExchange,
}

impl Fixture {
    pub(super) fn new(variables: &[(&str, &str)]) -> Self {
        let environment_map: BTreeMap<String, String> = variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        let oidc = Arc::new(RecordingOidc::default());
        let recorded_oidc: Arc<RecordingOidc> = Arc::clone(&oidc);
        let oidc_transport: Arc<dyn OidcTransport> = recorded_oidc;
        let exchange = AzureOidcExchange::new(
            Settings {
                azure_authority_host: "https://login.contoso.test".to_owned(),
            },
            environment_map.clone(),
            oidc_transport,
            Arc::new(FixedOidcClock),
        )
        .unwrap();
        Self {
            environment: ManagedIdentityEnvironment::from_lookup(&environment_map),
            host: FixtureHost {
                environment: environment_map,
                files: BTreeMap::new(),
                platform: "linux".to_owned(),
            },
            imds: RecordingImds::default(),
            scheduler: RecordingScheduler::default(),
            ids: FixedIds,
            clock: FixedClock::fixed(NOW),
            hosted: RecordingHosted::default(),
            arc: RecordingArc::default(),
            files: FixtureFiles::default(),
            oidc,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap(),
            exchange,
        }
    }

    pub(super) fn with_file(mut self, path: &str, content: &str) -> Self {
        self.files.insert(path, content);
        self.host.files.insert(path.to_owned(), content.to_owned());
        self
    }

    pub(super) fn io(&self) -> ManagedIdentityIo<'_> {
        ManagedIdentityIo {
            imds: ImdsIo {
                transport: &self.imds,
                scheduler: &self.scheduler,
                ids: &self.ids,
                clock: &self.clock,
            },
            hosted: crate::azure_hosted_identity::request::HostedIo {
                transport: &self.hosted,
                clock: &self.clock,
                redactor: &self.redactor,
            },
            arc: &self.arc,
            workload: WorkloadIo {
                exchange: &self.exchange,
                files: &self.files,
            },
        }
    }

    pub(super) fn credential<'fixture>(
        &'fixture self,
        client_id: Option<&'fixture str>,
    ) -> ManagedIdentityCredential<'fixture, FixtureHost> {
        ManagedIdentityCredential {
            environment: &self.environment,
            host: &self.host,
            client_id,
            exclude_workload_identity: false,
            within_default_credential: false,
            probe: ProbePolicy::default(),
        }
    }

    pub(super) fn acquire(
        &self,
        scope: &[&str],
    ) -> Result<ManagedIdentityToken, ManagedIdentityError> {
        let credential = self.credential(None);
        credential.token(&self.io(), scope)
    }
}
