use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, UNIX_EPOCH};

use async_trait::async_trait;
use aws_smithy_async::time::{SharedTimeSource, StaticTimeSource};
use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::credentials::error::CredentialError;
use betterlite_bedrock::sagemaker_runtime::runtime::{
    ChunkStream, SagemakerEnvironment, SagemakerRuntime,
};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::compatible_wire::NEW_ID;
use betterlite_openai::sagemaker_completion::response::TextTokenCounter;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use betterlite_provider::data_cloud_dispatch::context::EnvironmentSource;
use betterlite_provider::enterprise_ml_dispatch::sagemaker::SagemakerDeps;
use betterlite_provider::enterprise_ml_dispatch::transport::{
    AwsCredentials, EnterpriseTransport, SharedCredentials, SharedTransport,
};
use betterlite_provider::sap_auth::token::SapClock;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_tokenizer::engine::error::EngineError;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use secrecy::SecretString;
use serde::Serialize;

use super::case::{Case, Reply};
use super::wire::{Failure, Sent, failure};

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the b8331 case needs no OIDC network"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the b8331 case needs no Azure token source"),
        })
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

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

fn body_bytes(body: RequestBody) -> Option<Vec<u8>> {
    match body {
        RequestBody::Json(data) | RequestBody::Bytes { data, .. } => Some(data),
        RequestBody::Empty | RequestBody::Multipart { .. } => None,
    }
}

// answers every outbound request from the case's replies, in order, and keeps what was sent
pub(super) struct Scripted {
    replies: Mutex<VecDeque<Reply>>,
    sent: Mutex<Vec<Sent>>,
}

impl Scripted {
    pub(super) fn new(replies: &[Reply]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().cloned().collect()),
            sent: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn keep(&self, sent: Sent) {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(sent);
    }

    // the request an arm keeps without sending, its body serialized as the provider wire form
    pub(super) fn keep_json<T>(
        &self,
        url: String,
        headers: HeaderMap,
        body: &T,
    ) -> Result<(), Failure>
    where
        T: Serialize,
    {
        let bytes = serde_json::to_vec(body).map_err(|error| failure(None, &error))?;
        self.keep(Sent {
            method: String::from("POST"),
            url,
            headers,
            body: Some(bytes),
        });
        Ok(())
    }

    pub(super) fn sent(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn take(&self) -> Result<UpstreamResponse, TransportError> {
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or(TransportError::Closed)?;
        Ok(UpstreamResponse {
            status: reply.status,
            headers: HeaderMap::new(),
            body: reply.body.into_bytes(),
        })
    }
}

#[async_trait]
impl EnterpriseTransport for Scripted {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let method = request.method.as_str().to_owned();
        let url = request.url.to_string();
        self.keep(Sent {
            method,
            url,
            headers: request.headers,
            body: body_bytes(request.body),
        });
        self.take()
    }

    async fn open_stream(
        &self,
        _request: OutboundRequest,
    ) -> Result<(u16, ChunkStream), TransportError> {
        Err(TransportError::Closed)
    }
}

pub(super) struct StubCredentials;

#[async_trait]
impl AwsCredentials for StubCredentials {
    async fn resolve(
        &self,
        _settings: AwsSettings,
        _region: String,
    ) -> Result<Credentials, CredentialError> {
        Ok(Credentials {
            access_key_id: SecretString::from(String::from("AKIAFIXTURE")),
            secret_access_key: SecretString::from(String::from("fixture-secret")),
            session_token: None,
        })
    }
}

pub(super) struct Counter;

impl TextTokenCounter for Counter {
    fn count_response_tokens(&self, text: &str) -> u64 {
        u64::try_from(text.split_whitespace().count()).unwrap_or_default()
    }
}

pub(super) struct NeverEncoding;

impl TextEncoding for NeverEncoding {
    fn count_tokens(&self, _text: &str) -> Result<usize, EngineError> {
        Err(EngineError::PoolClosed)
    }
}

pub(super) struct FixedClock;

impl SapClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap_or_default()
    }
}

pub(super) struct Fixture {
    pub(super) transport: Scripted,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) secrets: SecretLookup,
    pub(super) sap_clock: FixedClock,
    pub(super) prompts: PromptFactory,
    credentials: StubCredentials,
    counter: Counter,
    encoding: NeverEncoding,
    time: SharedTimeSource,
}

impl Fixture {
    pub(super) fn new(case: &Case) -> Self {
        Self {
            transport: Scripted::new(&case.upstream),
            environment: case.environment(),
            secrets: secrets(),
            sap_clock: FixedClock,
            prompts: PromptFactory {
                cache: Mutex::new(builtin_entries()),
                hub: Hub::Unreachable,
            },
            credentials: StubCredentials,
            counter: Counter,
            encoding: NeverEncoding,
            time: SharedTimeSource::new(StaticTimeSource::new(UNIX_EPOCH)),
        }
    }

    pub(super) const fn source(&self) -> EnvironmentSource<'_> {
        EnvironmentSource {
            secrets: &self.secrets,
            environment: &self.environment,
        }
    }

    fn sagemaker_environment(&self) -> SagemakerEnvironment {
        SagemakerEnvironment {
            region_name: self.environment.get("AWS_REGION_NAME").cloned(),
            region: self.environment.get("AWS_REGION").cloned(),
            session_region: None,
        }
    }

    pub(super) fn deps(&self) -> SagemakerDeps<'_> {
        SagemakerDeps {
            transport: &self.transport,
            credentials: &self.credentials,
            environment: self.sagemaker_environment(),
            clock: self.time.clone(),
            counter: &self.counter,
            encoding: &self.encoding,
            prompts: &self.prompts,
            timeout: Duration::from_secs(5),
            identifier: NEW_ID,
        }
    }

    pub(super) fn runtime(
        &self,
    ) -> SagemakerRuntime<'_, SharedTransport<'_>, SharedCredentials<'_>> {
        SagemakerRuntime {
            transport: SharedTransport {
                inner: &self.transport,
            },
            credentials: SharedCredentials {
                inner: &self.credentials,
            },
            environment: self.sagemaker_environment(),
            clock: self.time.clone(),
            counter: &self.counter,
            timeout: Duration::from_secs(5),
            identifier: NEW_ID,
        }
    }
}
