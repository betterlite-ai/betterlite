use std::future::Future;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::realtime_http_calls::RealtimeHttpCalls;
use betterlite_core::api::realtime_http_calls::call::{
    CallCommon, ClientSecretCall, RealtimeCallsCall, SuppliedClient, TranscriptionSessionCall,
};
use betterlite_core::api::realtime_http_calls::dispatch::{
    CallsRequest, RealtimeHttpDispatch, RealtimeHttpRequest, SessionEndpoint, SessionRequest,
};
use betterlite_core::api::realtime_http_calls::error::RealtimeHttpCallsError;
use betterlite_core::api::realtime_http_calls::provider::RealtimeHttpConfig;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
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
use secrecy::SecretString;

use super::convert::{CallsArguments, ClientSecretArguments, Common, TranscriptionArguments};
use super::session::RealtimeSession;

// The provider transport behind the SDK's dispatch seam, with this session's pooled client and the recording pre-call
pub(crate) struct Dispatcher<'session> {
    session: &'session RealtimeSession,
}

impl<'session> Dispatcher<'session> {
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

    async fn perform(
        &self,
        request: RealtimeHttpRequest<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpError> {
        let env = self.session.pre_post();
        let mut log = LogContext {
            state: logging,
            env: &env,
        };
        let endpoint = endpoint_request(request);
        let deps = self.deps(endpoint.config, endpoint.supplied);
        match endpoint.call {
            EndpointCall::Session(call) => session_post(&deps, &call, &mut log).await,
            EndpointCall::Calls(call) => calls_post(&deps, &call, &mut log).await,
        }
    }
}

impl RealtimeClientSource for Dispatcher<'_> {
    fn client(&self) -> Result<ProxiedClient, ClientError> {
        Ok(self.session.client.clone())
    }
}

impl RealtimeHttpDispatch for Dispatcher<'_> {
    type Error = RealtimeHttpError;

    fn exchange(
        &self,
        request: RealtimeHttpRequest<'_>,
        logging: &mut LoggingState,
    ) -> impl Future<Output = Result<UpstreamResponse, RealtimeHttpError>> + Send {
        self.perform(request, logging)
    }
}

// The one provider call an SDK request carries, with what its transport deps take
enum EndpointCall<'call> {
    Session(SessionCall<'call>),
    Calls(CallsCall<'call>),
}

struct EndpointRequest<'call> {
    call: EndpointCall<'call>,
    config: Option<&'call RealtimeHttpConfig>,
    supplied: Option<SuppliedClient<'call>>,
}

fn endpoint_request(request: RealtimeHttpRequest<'_>) -> EndpointRequest<'_> {
    match request {
        RealtimeHttpRequest::Session(session) => EndpointRequest {
            call: EndpointCall::Session(SessionCall {
                endpoint: http_endpoint(session.endpoint),
                api_base: session.api_base,
                api_key: session.api_key,
                request_data: session.request_data,
                model: Some(session.model),
                extra_headers: session.extra_headers,
                api_version: session.api_version,
                timeout: Some(session.timeout),
            }),
            config: session.provider,
            supplied: session.client,
        },
        RealtimeHttpRequest::Calls(calls) => EndpointRequest {
            call: EndpointCall::Calls(CallsCall {
                api_base: calls.api_base,
                ephemeral_key: calls.ephemeral_key,
                sdp: calls.sdp,
                model: Some(calls.model),
                session: calls.session,
                extra_headers: calls.extra_headers,
                api_version: calls.api_version,
                timeout: Some(calls.timeout),
            }),
            config: calls.provider,
            supplied: calls.client,
        },
    }
}

const fn http_endpoint(endpoint: SessionEndpoint) -> HttpEndpoint {
    match endpoint {
        SessionEndpoint::ClientSecrets => HttpEndpoint::ClientSecrets,
        SessionEndpoint::TranscriptionSessions => HttpEndpoint::TranscriptionSessions,
    }
}

fn call_common<'call>(
    arguments: &'call Common,
    api_key: Option<&'call SecretString>,
) -> CallCommon<'call> {
    CallCommon {
        model: arguments.model.as_deref(),
        timeout: arguments.timeout,
        api_base: arguments.api_base.as_deref(),
        api_key,
        api_version: arguments.api_version.as_deref(),
        extra_headers: arguments.extra_headers.as_ref(),
        client: None,
        kwargs: &arguments.kwargs,
    }
}

fn resolution(session: &RealtimeSession) -> ResolutionContext<'_> {
    ResolutionContext {
        catalog: &session.catalog,
        known_model_names: &session.known_model_names,
        custom_providers: &session.custom_providers,
        environment: &session.environment,
        registry: session.registry.clone(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

// The three endpoints' prepared arguments, so one exchange carries the logging state and the SDK dispatch shared by all of them
pub(crate) enum Prepared {
    ClientSecret(ClientSecretArguments),
    Transcription(TranscriptionArguments),
    Calls(CallsArguments),
}

impl Prepared {
    const fn call_type(&self) -> &'static str {
        match self {
            Self::ClientSecret(_) => "arealtime_client_secret",
            Self::Transcription(_) => "arealtime_transcription_session",
            Self::Calls(_) => "arealtime_calls",
        }
    }

    fn sdk_call(&self) -> SdkCall<'_> {
        match self {
            Self::ClientSecret(arguments) => SdkCall::ClientSecret(ClientSecretCall {
                common: call_common(&arguments.common, arguments.common.api_key.as_ref()),
                session: arguments.session.as_ref(),
                expires_after: arguments.expires_after.as_ref(),
            }),
            Self::Transcription(arguments) => SdkCall::Transcription(TranscriptionSessionCall {
                common: call_common(&arguments.common, arguments.common.api_key.as_ref()),
                transcription_session: arguments.transcription_session.as_ref(),
            }),
            Self::Calls(arguments) => SdkCall::Calls(RealtimeCallsCall {
                common: call_common(&arguments.common, arguments.common.api_key.as_ref()),
                openai_ephemeral_key: arguments.ephemeral_key.as_str(),
                sdp_body: arguments.sdp.as_slice(),
                session: arguments.session.as_ref(),
            }),
        }
    }
}

// The SDK call each prepared endpoint names, borrowing the arguments until the exchange answers
enum SdkCall<'call> {
    ClientSecret(ClientSecretCall<'call>),
    Transcription(TranscriptionSessionCall<'call>),
    Calls(RealtimeCallsCall<'call>),
}

pub(crate) async fn exchange(
    session: &RealtimeSession,
    prepared: &Prepared,
) -> Result<UpstreamResponse, RealtimeHttpCallsError<RealtimeHttpError>> {
    let mut logging = session
        .logging(prepared.call_type())
        .map_err(RealtimeHttpCallsError::Logging)?;
    let dispatcher = Dispatcher { session };
    let calls = RealtimeHttpCalls {
        dispatch: &dispatcher,
        settings: &session.http_settings,
        resolution: resolution(session),
    };
    match prepared.sdk_call() {
        SdkCall::ClientSecret(call) => {
            calls
                .acreate_realtime_client_secret(call, &mut logging)
                .await
        }
        SdkCall::Transcription(call) => {
            calls
                .acreate_realtime_transcription_session(call, &mut logging)
                .await
        }
        SdkCall::Calls(call) => calls.arealtime_calls(call, &mut logging).await,
    }
}

mod tests;
