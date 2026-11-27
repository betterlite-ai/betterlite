use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;

use betterlite_config::api::realtime_http_calls::load_settings;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::realtime_http_calls::RealtimeHttpCalls;
use betterlite_core::api::realtime_http_calls::call::{
    CallCommon, ClientSecretCall, RealtimeCallsCall, SuppliedClient, TranscriptionSessionCall,
};
use betterlite_core::api::realtime_http_calls::dispatch::{
    RealtimeHttpDispatch, RealtimeHttpRequest, SessionEndpoint as SdkEndpoint,
};
use betterlite_core::api::realtime_http_calls::error::RealtimeHttpCallsError;
use betterlite_core::api::realtime_http_calls::provider::RealtimeHttpConfig;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_provider::api::realtime_http_transport::TransportDeps;
use betterlite_provider::api::realtime_http_transport::calls::{CallsCall, calls_post};
use betterlite_provider::api::realtime_http_transport::client::{
    RealtimeClientSource, SuppliedClient as ProviderClient,
};
use betterlite_provider::api::realtime_http_transport::error::RealtimeHttpError;
use betterlite_provider::api::realtime_http_transport::session::{
    SessionCall, SessionEndpoint as HttpEndpoint, session_post,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::result::{logged, outcome};
use super::support::{Sink, logging_state, recording_env};
use super::{Call, Operation};

// The production SDK over the mock's base: resolution, the loaded globals and B2456's handlers, as a caller runs them
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let document = serde_json::to_string(&BTreeMap::from([("litellm_settings", &call.settings)]))
        .map_err(SdkHttpError::Serialize)?;
    let settings = load_settings(&document, env.clone())
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let catalog = BundledCatalog::default().get();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let dispatch = Dispatcher::new(env)?;
    let mut state = logging_state()?;
    let calls = RealtimeHttpCalls {
        dispatch: &dispatch,
        settings: &settings,
        resolution: ResolutionContext {
            catalog: &catalog,
            known_model_names: &known,
            custom_providers: &custom,
            environment: env,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        },
    };
    let answer = run(&calls, call, api_base, &mut state).await;
    Ok(SdkResult::RealtimeHttpCalls {
        value: Box::new(outcome(&answer, logged(&state))?),
    })
}

// Each SDK method's declared arguments over an empty kwargs map a borrow keeps alive for this block
async fn run(
    sdk: &RealtimeHttpCalls<'_, Dispatcher>,
    call: &Call,
    api_base: &str,
    log: &mut LoggingState,
) -> Result<UpstreamResponse, RealtimeHttpCallsError<RealtimeHttpError>> {
    let api_key = call.api_key.as_deref().map(SecretString::from);
    let common = CallCommon {
        model: call.model.as_deref(),
        timeout: None,
        api_base: Some(api_base),
        api_key: api_key.as_ref(),
        api_version: call.api_version.as_deref(),
        extra_headers: call.extra_headers.as_ref(),
        client: None,
        kwargs: &IndexMap::new(),
    };
    match call.operation {
        Operation::ClientSecret => {
            let req = ClientSecretCall {
                common,
                session: call.session.as_ref(),
                expires_after: call.expires_after.as_ref(),
            };
            sdk.acreate_realtime_client_secret(req, log).await
        }
        Operation::TranscriptionSession => {
            let req = TranscriptionSessionCall {
                common,
                transcription_session: call.transcription_session.as_ref(),
            };
            sdk.acreate_realtime_transcription_session(req, log).await
        }
        Operation::Calls => {
            let req = RealtimeCallsCall {
                common,
                openai_ephemeral_key: &call.ephemeral_key,
                sdp_body: &call.sdp,
                session: call.session.as_ref(),
            };
            sdk.arealtime_calls(req, log).await
        }
    }
}

// The provider transport behind the SDK's dispatch seam, with the pooled client and the recording pre-call
struct Dispatcher {
    pool: ClientPool,
    tls: TlsPolicy,
    routing: Arc<ProxyPolicy>,
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
}

impl Dispatcher {
    fn new(env: &BTreeMap<String, String>) -> Result<Self, SdkHttpError> {
        let config = ConfigLoader {
            lookup: env.clone(),
        }
        .load("model_list: []")
        .map_err(|error| SdkHttpError::ConfigLoad(Box::new(error)))?;
        let routing = Arc::new(
            ProxyPolicy::resolve(&ProxyTrust::default(), env).map_err(SdkHttpError::ProxyPolicy)?,
        );
        Ok(Self {
            pool: ClientPool::new(1, TransportLimits::DEFAULT),
            tls: TlsPolicy::resolve(&config, env, None)
                .map_err(|error| SdkHttpError::Invalid(error.to_string()))?,
            routing,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?,
            sink: Sink,
            now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
                .map_err(|error| SdkHttpError::Invalid(error.to_string()))?,
        })
    }

    fn deps<'deps>(
        &'deps self,
        config: Option<&'deps RealtimeHttpConfig>,
        supplied: Option<SuppliedClient<'deps>>,
    ) -> TransportDeps<'deps, RealtimeHttpConfig> {
        TransportDeps {
            config,
            source: self,
            supplied: supplied
                .map(|SuppliedClient { kind, transport }| ProviderClient { kind, transport }),
            limits: ApiLimits::default(),
        }
    }

    // The conversion into B2456's call records, one branch per handler.
    async fn perform(
        &self,
        request: RealtimeHttpRequest<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpError> {
        let record_env = recording_env(&self.redactor, &self.sink, self.now);
        let mut log = LogContext {
            state: logging,
            env: &record_env,
        };
        match request {
            RealtimeHttpRequest::Session(session) => {
                let call = SessionCall {
                    endpoint: match session.endpoint {
                        SdkEndpoint::ClientSecrets => HttpEndpoint::ClientSecrets,
                        SdkEndpoint::TranscriptionSessions => HttpEndpoint::TranscriptionSessions,
                    },
                    api_base: session.api_base,
                    api_key: session.api_key,
                    request_data: session.request_data,
                    model: Some(session.model),
                    extra_headers: session.extra_headers,
                    api_version: session.api_version,
                    timeout: Some(session.timeout),
                };
                let deps = self.deps(session.provider, session.client);
                session_post(&deps, &call, &mut log).await
            }
            RealtimeHttpRequest::Calls(calls) => {
                let call = CallsCall {
                    api_base: calls.api_base,
                    ephemeral_key: calls.ephemeral_key,
                    sdp: calls.sdp,
                    model: Some(calls.model),
                    session: calls.session,
                    extra_headers: calls.extra_headers,
                    api_version: calls.api_version,
                    timeout: Some(calls.timeout),
                };
                calls_post(&self.deps(calls.provider, calls.client), &call, &mut log).await
            }
        }
    }
}

impl RealtimeClientSource for Dispatcher {
    fn client(&self) -> Result<ProxiedClient, ClientError> {
        ClientPool::client(&self.pool, &self.tls, &self.routing).map_err(ClientError::Pool)
    }
}

impl RealtimeHttpDispatch for Dispatcher {
    type Error = RealtimeHttpError;

    fn exchange(
        &self,
        request: RealtimeHttpRequest<'_>,
        logging: &mut LoggingState,
    ) -> impl Future<Output = Result<UpstreamResponse, RealtimeHttpError>> + Send {
        self.perform(request, logging)
    }
}
