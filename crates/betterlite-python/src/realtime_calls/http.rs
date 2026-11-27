use std::future::Future;

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
use betterlite_logging::logging_state::LoggingState;
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

pub(crate) async fn client_secret(
    session: &RealtimeSession,
    arguments: &ClientSecretArguments,
) -> Result<UpstreamResponse, RealtimeHttpCallsError<RealtimeHttpError>> {
    let mut logging = session
        .logging("arealtime_client_secret")
        .map_err(RealtimeHttpCallsError::Logging)?;
    let dispatcher = Dispatcher { session };
    let common = call_common(&arguments.common, arguments.common.api_key.as_ref());
    let calls = RealtimeHttpCalls {
        dispatch: &dispatcher,
        settings: &session.http_settings,
        resolution: session.resolution(),
    };
    calls
        .acreate_realtime_client_secret(
            ClientSecretCall {
                common,
                session: arguments.session.as_ref(),
                expires_after: arguments.expires_after.as_ref(),
            },
            &mut logging,
        )
        .await
}

pub(crate) async fn transcription(
    session: &RealtimeSession,
    arguments: &TranscriptionArguments,
) -> Result<UpstreamResponse, RealtimeHttpCallsError<RealtimeHttpError>> {
    let mut logging = session
        .logging("arealtime_transcription_session")
        .map_err(RealtimeHttpCallsError::Logging)?;
    let dispatcher = Dispatcher { session };
    let common = call_common(&arguments.common, arguments.common.api_key.as_ref());
    let calls = RealtimeHttpCalls {
        dispatch: &dispatcher,
        settings: &session.http_settings,
        resolution: session.resolution(),
    };
    calls
        .acreate_realtime_transcription_session(
            TranscriptionSessionCall {
                common,
                transcription_session: arguments.transcription_session.as_ref(),
            },
            &mut logging,
        )
        .await
}

pub(crate) async fn calls(
    session: &RealtimeSession,
    arguments: &CallsArguments,
) -> Result<UpstreamResponse, RealtimeHttpCallsError<RealtimeHttpError>> {
    let mut logging = session
        .logging("arealtime_calls")
        .map_err(RealtimeHttpCallsError::Logging)?;
    let dispatcher = Dispatcher { session };
    let common = call_common(&arguments.common, arguments.common.api_key.as_ref());
    let calls = RealtimeHttpCalls {
        dispatch: &dispatcher,
        settings: &session.http_settings,
        resolution: session.resolution(),
    };
    calls
        .arealtime_calls(
            RealtimeCallsCall {
                common,
                openai_ephemeral_key: arguments.ephemeral_key.as_str(),
                sdp_body: arguments.sdp.as_slice(),
                session: arguments.session.as_ref(),
            },
            &mut logging,
        )
        .await
}
