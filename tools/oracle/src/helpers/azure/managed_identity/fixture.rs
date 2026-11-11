use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_azure::azure_arc_identity::{ArcRequest, ArcResponse, ArcTransport};
use betterlite_azure::azure_certificate::assertion::IdSource;
use betterlite_azure::azure_client_secret::transport::Clock;
use betterlite_azure::azure_hosted_identity::request::{
    HostedRequest, HostedResponse, HostedTransport,
};
use betterlite_azure::azure_managed_identity::ManagedIdentityCredential;
use betterlite_azure::azure_managed_identity::environment::ManagedIdentityEnvironment;
use betterlite_azure::azure_managed_identity::imds::{
    ImdsRequest, ImdsResponse, ImdsTransport, ProbePolicy,
};
use betterlite_azure::azure_managed_identity::runtime::{
    ImdsIo, ManagedIdentityIo, Scheduler, TokenFiles, WorkloadIo,
};
use betterlite_azure::oidc_exchange::AzureOidcExchange;
use betterlite_azure::oidc_exchange::transport::{
    Clock as OidcClock, HttpReply, Transport as OidcTransport,
};
use betterlite_config::sections::azure_oidc_exchange::Settings;
use betterlite_redact::error::RedactionError;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

const REQUEST_ID: &str = "00000000-0000-4000-8000-000000000001";
const DEFAULT_AUTHORITY: &str = "https://login.microsoftonline.com";
const KEPT_HEADERS: [&str; 6] = [
    "metadata",
    "secret",
    "x-identity-header",
    "x-client-sku",
    "x-client-ver",
    "x-ms-client-request-id",
];

#[derive(Clone, Deserialize)]
pub(crate) struct Reply {
    status: u16,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: String,
}

pub(crate) struct Shared {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Value>>,
    sleeps: Mutex<Vec<f64>>,
}

impl Shared {
    pub(crate) fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
            sleeps: Mutex::new(Vec::new()),
        })
    }

    fn next(&self) -> io::Result<Reply> {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| io::Error::other("no reply queued"))
    }

    pub(crate) fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn sleeps(&self) -> Vec<f64> {
        self.sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn header_map<I>(headers: I) -> BTreeMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    headers
        .into_iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .filter(|(name, _)| KEPT_HEADERS.contains(&name.as_str()))
        .collect()
}

fn record(shared: &Shared, request: Value) {
    shared
        .requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request);
}

pub(crate) struct SharedTransport {
    shared: Arc<Shared>,
}

impl ImdsTransport for SharedTransport {
    fn get(&self, request: &ImdsRequest) -> io::Result<ImdsResponse> {
        record(
            &self.shared,
            json!({
                "method": "GET",
                "url": request.url,
                "params": BTreeMap::from_iter(request.query.iter().cloned()),
                "headers": header_map(request.headers.iter().cloned()),
                "form": null,
                "timeout": request.timeout_seconds,
            }),
        );
        let reply = self.shared.next()?;
        Ok(ImdsResponse {
            status: reply.status,
            headers: reply.headers.into_iter().collect(),
            body: SecretString::from(reply.body),
        })
    }
}

impl OidcTransport for SharedTransport {
    fn post_form(&self, url: &str, form: &[(String, SecretString)]) -> io::Result<HttpReply> {
        let recorded_form: BTreeMap<String, String> = form
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect();
        record(
            &self.shared,
            json!({
                "method": "POST",
                "url": url,
                "params": BTreeMap::<String, String>::new(),
                "headers": BTreeMap::<String, String>::new(),
                "form": recorded_form,
                "timeout": null,
            }),
        );
        let reply = self.shared.next()?;
        Ok(HttpReply {
            status: reply.status,
            body: SecretString::from(reply.body),
        })
    }
}

// the oracle cases reach hosted protocols through selection only, so a request here is a fixture bug
impl HostedTransport for SharedTransport {
    fn send(&self, _request: &HostedRequest) -> io::Result<HostedResponse> {
        Err(io::Error::other(
            "hosted identity is not part of the managed identity cases",
        ))
    }
}

impl ArcTransport for SharedTransport {
    fn get(&self, _request: &ArcRequest) -> io::Result<ArcResponse> {
        Err(io::Error::other(
            "Arc identity is not part of the managed identity cases",
        ))
    }
}

pub(crate) struct FixedOidcClock {
    now: f64,
}

impl OidcClock for FixedOidcClock {
    fn now_unix_seconds(&self) -> f64 {
        self.now
    }
}

pub(crate) struct Fixture {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) files: BTreeMap<String, String>,
    pub(crate) now: i64,
    pub(crate) shared: Arc<Shared>,
    identity: ManagedIdentityEnvironment,
    transport: Arc<SharedTransport>,
    exchange: AzureOidcExchange,
    redactor: Redactor,
}

impl Fixture {
    pub(crate) fn new(
        environment: &BTreeMap<String, String>,
        files: &BTreeMap<String, String>,
        now: i64,
        shared: Arc<Shared>,
    ) -> Result<Self, FixtureError> {
        let transport = Arc::new(SharedTransport {
            shared: Arc::clone(&shared),
        });
        let recorded: Arc<SharedTransport> = Arc::clone(&transport);
        let oidc_transport: Arc<dyn OidcTransport> = recorded;
        let now_seconds = f64::from(i32::try_from(now).map_err(|_error| FixtureError::Clock)?);
        let exchange = AzureOidcExchange::new(
            Settings {
                azure_authority_host: environment
                    .get("AZURE_AUTHORITY_HOST")
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_AUTHORITY.to_owned()),
            },
            environment.clone(),
            oidc_transport,
            Arc::new(FixedOidcClock { now: now_seconds }),
        )
        .map_err(FixtureError::Exchange)?;
        Ok(Self {
            environment: environment.clone(),
            files: files.clone(),
            now,
            shared,
            identity: ManagedIdentityEnvironment::from_lookup(environment),
            transport,
            exchange,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH)?,
        })
    }

    pub(crate) const fn credential<'fixture>(
        &'fixture self,
        client_id: Option<&'fixture str>,
        exclude_workload_identity: bool,
        probe: ProbePolicy,
    ) -> ManagedIdentityCredential<'fixture, Self> {
        ManagedIdentityCredential {
            environment: &self.identity,
            host: self,
            client_id,
            exclude_workload_identity,
            within_default_credential: false,
            probe,
        }
    }

    pub(crate) fn io(&self) -> ManagedIdentityIo<'_> {
        ManagedIdentityIo {
            imds: ImdsIo {
                transport: self.transport.as_ref(),
                scheduler: self,
                ids: self,
                clock: self,
            },
            hosted: betterlite_azure::azure_hosted_identity::request::HostedIo {
                transport: self.transport.as_ref(),
                clock: self,
                redactor: &self.redactor,
            },
            arc: self.transport.as_ref(),
            workload: WorkloadIo {
                exchange: &self.exchange,
                files: self,
            },
        }
    }
}

impl Scheduler for Fixture {
    fn sleep(&self, delay: Duration) {
        self.shared
            .sleeps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(delay.as_secs_f64());
    }
}

impl Clock for Fixture {
    fn now_unix_seconds(&self) -> i64 {
        self.now
    }
}

impl IdSource for Fixture {
    fn next_id(&self) -> String {
        REQUEST_ID.to_owned()
    }
}

impl TokenFiles for Fixture {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        self.files
            .get(path)
            .map(|content| content.as_bytes().to_vec())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FixtureError {
    #[error(transparent)]
    Exchange(#[from] betterlite_azure::oidc_exchange::OidcExchangeError),
    #[error(transparent)]
    Redaction(#[from] RedactionError),
    #[error("the fixture clock is out of range")]
    Clock,
}
