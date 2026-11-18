use std::collections::BTreeMap;
use std::future::{Future, Ready, ready};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::manager::ManagerState;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_secret_managers::secret_read_dispatch::SecretManagerClient;
use betterlite_secret_managers::secret_read_dispatch::protocols::SyncSecretReader;
use betterlite_secret_managers::secret_read_dispatch::settings::SecretManagerSettings;
use betterlite_secret_managers::secret_types::{KeyManagementSystem, SecretAccessMode};
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, ReplyRequest, Timeout,
    TransferError,
};
use crate::credentials::error::CredentialError;

pub(super) const ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(super) const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
pub(super) const SESSION_TOKEN: &str = "session-token";

// 1_700_000_000 seconds is 2023-11-14T22:13:20Z, the instant the signing assertions name
pub(super) fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("the fixture clock is in range")
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(super) fn credentials(access: &str, secret_key: &str, token: Option<&str>) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access.to_owned()),
        secret_access_key: SecretString::from(secret_key.to_owned()),
        session_token: token.map(|value| SecretString::from(value.to_owned())),
    }
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the fixture runtime builds")
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the fixture JSON is an object")
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture JSON parses")
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) type Seen = Arc<Mutex<Vec<EmbeddingRequest>>>;

#[derive(Clone, Debug)]
pub(super) enum Reply {
    Answer(EmbeddingReply),
    Timeout,
    Failed,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self::status(200, &[], body)
    }

    pub(super) fn status(status: u16, pairs: &[(&str, &str)], body: &str) -> Self {
        Self::redirected(status, pairs, body, "POST", "")
    }

    // an empty url leaves the request url to the transport, which fills in the url it was sent
    pub(super) fn redirected(
        status: u16,
        pairs: &[(&str, &str)],
        body: &str,
        method: &str,
        url: &str,
    ) -> Self {
        Self::Answer(EmbeddingReply {
            status,
            headers: headers(pairs),
            body: body.as_bytes().to_vec(),
            request: ReplyRequest {
                method: method.to_owned(),
                url: url.to_owned(),
            },
        })
    }
}

// the fixture client stands in for both an HTTPHandler and an AsyncHTTPHandler and records the sent request
pub(super) struct FixtureClient {
    pub(super) reply: Reply,
    pub(super) seen: Seen,
}

impl FixtureClient {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            seen: Seen::default(),
        }
    }

    pub(super) fn requests(&self) -> Vec<EmbeddingRequest> {
        self.seen.lock().unwrap().clone()
    }
}

impl EmbeddingTransport for FixtureClient {
    fn post(
        &self,
        request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send {
        let reply = match &self.reply {
            Reply::Answer(reply) => {
                let mut answer = reply.clone();
                if answer.request.url.is_empty() {
                    answer.request.url = request.url.clone();
                }
                Ok(answer)
            }
            Reply::Timeout => Err(TransferError::Timeout),
            Reply::Failed => Err(TransferError::Failed(TransportError::Closed)),
        };
        self.seen.lock().unwrap().push(request);
        ready(reply)
    }
}

pub(super) struct Factory {
    pub(super) reply: Reply,
    pub(super) seen: Seen,
    pub(super) sync_calls: Mutex<Vec<Option<Timeout>>>,
    pub(super) async_calls: Mutex<Vec<(String, Option<Timeout>)>>,
}

impl Factory {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            seen: Seen::default(),
            sync_calls: Mutex::new(Vec::new()),
            async_calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn sync_calls(&self) -> Vec<Option<Timeout>> {
        self.sync_calls.lock().unwrap().clone()
    }

    pub(super) fn async_calls(&self) -> Vec<(String, Option<Timeout>)> {
        self.async_calls.lock().unwrap().clone()
    }

    pub(super) fn requests(&self) -> Vec<EmbeddingRequest> {
        self.seen.lock().unwrap().clone()
    }
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, timeout: Option<Timeout>) -> Self::Client {
        self.sync_calls.lock().unwrap().push(timeout);
        FixtureClient {
            reply: self.reply.clone(),
            seen: Arc::clone(&self.seen),
        }
    }

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client {
        self.async_calls
            .lock()
            .unwrap()
            .push((provider.to_owned(), timeout));
        FixtureClient {
            reply: self.reply.clone(),
            seen: Arc::clone(&self.seen),
        }
    }
}

pub(super) struct Source {
    pub(super) credentials: Credentials,
    pub(super) calls: Mutex<Vec<(AwsSettings, String)>>,
}

impl Source {
    pub(super) fn new(credentials: Credentials) -> Self {
        Self {
            credentials,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<(AwsSettings, String)> {
        self.calls.lock().unwrap().clone()
    }

    pub(super) fn resolve<'source>(
        &'source self,
    ) -> impl FnOnce(&AwsSettings, &str) -> Ready<Result<Credentials, CredentialError>> + 'source
    {
        move |settings: &AwsSettings, region: &str| {
            self.calls
                .lock()
                .unwrap()
                .push((settings.clone(), region.to_owned()));
            ready(Ok(self.credentials.clone()))
        }
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secret_lookup(manager: Option<ManagerState>) -> SecretLookup {
    SecretLookup {
        manager,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

// a Vault reader records the thread it was read on and answers the region secret
pub(super) struct ManagedReader {
    region: String,
    reads: Mutex<Vec<(String, bool)>>,
}

impl ManagedReader {
    pub(super) fn new(region: &str) -> Self {
        Self {
            region: region.to_owned(),
            reads: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn reads(&self) -> Vec<(String, bool)> {
        self.reads.lock().unwrap().clone()
    }
}

impl SyncSecretReader for ManagedReader {
    fn sync_read_secret(&self, secret_name: &str) -> Option<SecretString> {
        let on_runtime = tokio::runtime::Handle::try_current().is_ok();
        self.reads
            .lock()
            .unwrap()
            .push((secret_name.to_owned(), on_runtime));
        assert!(
            !on_runtime,
            "a managed secret read must not run on the tokio runtime"
        );
        (secret_name == "AWS_REGION_NAME").then(|| SecretString::from(self.region.clone()))
    }
}

pub(super) fn managed_secrets(reader: Arc<ManagedReader>) -> SecretLookup {
    secret_lookup(Some(ManagerState {
        client: SecretManagerClient::Vault(reader),
        settings: Some(SecretManagerSettings::default()),
        system: Some(KeyManagementSystem::HashicorpVault),
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }))
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

// caller options reach the loader as raw JSON values, as the request body carries them
pub(super) fn raw(pairs: &[(&str, &str)]) -> BTreeMap<String, Box<RawValue>> {
    pairs
        .iter()
        .map(|(name, value)| {
            let text = serde_json::to_string(value).expect("the fixture value writes as JSON");
            (
                (*name).to_owned(),
                RawValue::from_string(text).expect("the fixture value is raw JSON"),
            )
        })
        .collect()
}
