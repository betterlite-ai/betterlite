use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use betterlite_config::api::realtime_http_calls::load_settings;
use betterlite_config::loader::ConfigLoader;
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::realtime_http_calls::RealtimeHttpCalls;
use betterlite_core::api::realtime_http_calls::call::{
    ClientSecretCall, RealtimeCallsCall, SuppliedClient, TranscriptionSessionCall,
};
use betterlite_core::api::realtime_http_calls::dispatch::{
    CallsRequest, RealtimeHttpDispatch, RealtimeHttpRequest, SessionEndpoint, SessionRequest,
};
use betterlite_core::api::realtime_http_calls::error::RealtimeHttpCallsError;
use betterlite_core::api::realtime_http_calls::provider::RealtimeHttpConfig;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_provider::api::realtime_http_transport::TransportDeps;
use betterlite_provider::api::realtime_http_transport::calls::{CallsCall, calls_post};
use betterlite_provider::api::realtime_http_transport::client::SuppliedClient as ProviderClient;
use betterlite_provider::api::realtime_http_transport::error::RealtimeHttpError;
use betterlite_provider::api::realtime_http_transport::session::{
    SessionCall, SessionEndpoint as ProviderEndpoint, session_post,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::SecretString;

use super::super::error::SdkHttpError;
use super::super::result::kinds::SdkResult;
use super::result::{logged, outcome};
use super::support::{Sink, Source, logging_state, now, recording_env, source};
use super::{Call, Operation};

// The production SDK over the mock's base: resolution, the loaded globals and B2456's handlers, as a caller runs them
pub(super) async fn execute(
    call: &Call,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<SdkResult, SdkHttpError> {
    let document = call.settings.document().map_err(SdkHttpError::Serialize)?;
    let settings = load_settings(&document, env.clone())
        .map_err(|error| SdkHttpError::Invalid(error.to_string()))?;
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let dispatch = Dispatcher::new(env)?;
    let mut state = logging_state()?;
    let calls = RealtimeHttpCalls {
        dispatch: &dispatch,
        settings: &settings,
        resolution: context(catalog.as_ref(), &known, &custom, env),
    };
    let answer = run(&calls, call, api_base, &mut state).await;
    let reported = outcome(&answer, logged(&state));
    Ok(SdkResult::RealtimeHttpCalls {
        value: Box::new(reported?),
    })
}

// The resolver's inputs: the bundled catalog, no declared names, and the case env the globals are absent from
fn context<'context>(
    catalog: &'context ModelCatalog,
    known: &'context BTreeSet<String>,
    custom: &'context BTreeSet<String>,
    env: &'context BTreeMap<String, String>,
) -> ResolutionContext<'context> {
    ResolutionContext {
        catalog,
        known_model_names: known,
        custom_providers: custom,
        environment: env,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

// The declared arguments and the kwargs each SDK method receives, borrowed for one call
struct Inputs<'call> {
    call: &'call Call,
    api_base: &'call str,
    api_key: Option<&'call SecretString>,
    kwargs: &'call IndexMap<String, Json>,
}

async fn run(
    calls: &RealtimeHttpCalls<'_, Dispatcher>,
    call: &Call,
    api_base: &str,
    state: &mut LoggingState,
) -> Result<UpstreamResponse, RealtimeHttpCallsError<RealtimeHttpError>> {
    let kwargs = IndexMap::new();
    let api_key = call.api_key.as_deref().map(SecretString::from);
    let inputs = Inputs {
        call,
        api_base,
        api_key: api_key.as_ref(),
        kwargs: &kwargs,
    };
    match call.operation {
        Operation::ClientSecret => {
            calls
                .acreate_realtime_client_secret(client_secret(&inputs), state)
                .await
        }
        Operation::TranscriptionSession => {
            calls
                .acreate_realtime_transcription_session(transcription(&inputs), state)
                .await
        }
        Operation::Calls => calls.arealtime_calls(realtime(&inputs), state).await,
    }
}

fn client_secret<'call>(inputs: &Inputs<'call>) -> ClientSecretCall<'call> {
    ClientSecretCall {
        model: inputs.call.model.as_deref(),
        session: inputs.call.session.as_ref(),
        expires_after: inputs.call.expires_after.as_ref(),
        timeout: None,
        api_base: Some(inputs.api_base),
        api_key: inputs.api_key,
        api_version: inputs.call.api_version.as_deref(),
        extra_headers: inputs.call.extra_headers.as_ref(),
        client: None,
        kwargs: inputs.kwargs,
    }
}

fn transcription<'call>(inputs: &Inputs<'call>) -> TranscriptionSessionCall<'call> {
    TranscriptionSessionCall {
        model: inputs.call.model.as_deref(),
        transcription_session: inputs.call.transcription_session.as_ref(),
        timeout: None,
        api_base: Some(inputs.api_base),
        api_key: inputs.api_key,
        api_version: inputs.call.api_version.as_deref(),
        extra_headers: inputs.call.extra_headers.as_ref(),
        client: None,
        kwargs: inputs.kwargs,
    }
}

fn realtime<'call>(inputs: &Inputs<'call>) -> RealtimeCallsCall<'call> {
    RealtimeCallsCall {
        openai_ephemeral_key: &inputs.call.ephemeral_key,
        sdp_body: &inputs.call.sdp,
        model: inputs.call.model.as_deref(),
        session: inputs.call.session.as_ref(),
        timeout: None,
        api_base: Some(inputs.api_base),
        api_key: inputs.api_key,
        api_version: inputs.call.api_version.as_deref(),
        extra_headers: inputs.call.extra_headers.as_ref(),
        client: None,
        kwargs: inputs.kwargs,
    }
}

// The provider transport behind the SDK's dispatch seam, with the pooled client and the recording pre-call
struct Dispatcher {
    source: Source,
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
        Ok(Self {
            source: source(&config, env)?,
            redactor: Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).map_err(SdkHttpError::Redactor)?,
            sink: Sink,
            now: now()?,
        })
    }

    fn deps<'deps>(
        &'deps self,
        config: Option<&'deps RealtimeHttpConfig>,
        supplied: Option<SuppliedClient<'deps>>,
    ) -> TransportDeps<'deps, RealtimeHttpConfig> {
        TransportDeps {
            config,
            source: &self.source,
            supplied: supplied.map(|client| ProviderClient {
                kind: client.kind,
                transport: client.transport,
            }),
            limits: ApiLimits::default(),
        }
    }
}

impl RealtimeHttpDispatch for Dispatcher {
    type Error = RealtimeHttpError;

    fn exchange(
        &self,
        request: RealtimeHttpRequest<'_>,
        logging: &mut LoggingState,
    ) -> impl Future<Output = Result<UpstreamResponse, RealtimeHttpError>> + Send {
        async move {
            let record_env = recording_env(&self.redactor, &self.sink, self.now);
            let mut log = LogContext {
                state: logging,
                env: &record_env,
            };
            match request {
                RealtimeHttpRequest::Session(session) => {
                    let call = session_call(&session);
                    session_post(
                        &self.deps(session.provider, session.client),
                        &call,
                        &mut log,
                    )
                    .await
                }
                RealtimeHttpRequest::Calls(calls) => {
                    let call = calls_call(&calls);
                    calls_post(&self.deps(calls.provider, calls.client), &call, &mut log).await
                }
            }
        }
    }
}

fn session_call<'call>(request: &'call SessionRequest<'call>) -> SessionCall<'call> {
    SessionCall {
        endpoint: match request.endpoint {
            SessionEndpoint::ClientSecrets => ProviderEndpoint::ClientSecrets,
            SessionEndpoint::TranscriptionSessions => ProviderEndpoint::TranscriptionSessions,
        },
        api_base: request.api_base,
        api_key: request.api_key,
        request_data: request.request_data,
        model: Some(request.model),
        extra_headers: request.extra_headers,
        api_version: request.api_version,
        timeout: Some(request.timeout),
    }
}

fn calls_call<'call>(request: &'call CallsRequest<'call>) -> CallsCall<'call> {
    CallsCall {
        api_base: request.api_base,
        ephemeral_key: request.ephemeral_key,
        sdp: request.sdp,
        model: Some(request.model),
        session: request.session,
        extra_headers: request.extra_headers,
        api_version: request.api_version,
        timeout: Some(request.timeout),
    }
}
