use std::{error::Error as StdError, time::Duration};

use betterlite_config::api::realtime_http_calls::RealtimeHttpCallSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, ResolvedModel, resolve};
use indexmap::IndexMap;

pub mod body;
pub mod call;
pub mod dispatch;
pub mod error;
pub mod logging;
pub mod provider;

#[path = "realtime_http_calls/tests.rs"]
mod tests;

use self::SessionOperation::{ClientSecret, Transcription};
use self::call::{CallCommon, ClientSecretCall, RealtimeCallsCall, TranscriptionSessionCall};
use self::dispatch::{
    CallsRequest, RealtimeHttpDispatch, RealtimeHttpRequest, SessionEndpoint, SessionRequest,
};
use self::error::{RealtimeHttpCallsError, RecordError};
use self::logging::{log_pre_call, params};
use self::provider::{RealtimeHttpCredentials, credentials};
use crate::api::realtime_contracts::records::{
    RealtimeExpiresAfter, RealtimeSessionConfig, TranscriptionSessionRequest,
};

// The two routing defaults the SDK applies before get_llm_provider.
pub const DEFAULT_REALTIME_MODEL: &str = "gpt-4o-realtime-preview";
pub const DEFAULT_TRANSCRIPTION_MODEL: &str = "gpt-realtime-whisper";

// acreate_realtime_client_secret, acreate_realtime_transcription_session and arealtime_calls over the deps a caller supplies once.
pub struct RealtimeHttpCalls<'deps, Dispatch: RealtimeHttpDispatch> {
    pub dispatch: &'deps Dispatch,
    pub settings: &'deps RealtimeHttpCallSettings,
    pub resolution: ResolutionContext<'deps>,
}

struct SessionPlan<'call> {
    common: &'call CallCommon<'call>,
    model_name: String,
    endpoint: SessionEndpoint,
    optional_params: IndexMap<String, Json>,
    records: SessionRecords,
}

enum SessionRecords {
    ClientSecret {
        session: Option<RealtimeSessionConfig>,
        expires_after: Option<RealtimeExpiresAfter>,
    },
    Transcription(TranscriptionSessionRequest),
}

impl SessionRecords {
    fn body(&self, model: &str) -> Result<IndexMap<String, Json>, RecordError> {
        match self {
            Self::ClientSecret {
                session,
                expires_after,
            } => body::client_secret(session.as_ref(), expires_after.as_ref(), model),
            Self::Transcription(record) => body::transcription_body(record, model),
        }
    }
}

enum SessionOperation<'call> {
    ClientSecret(&'call ClientSecretCall<'call>),
    Transcription(&'call TranscriptionSessionCall<'call>),
}

impl<DispatchError, Dispatch> RealtimeHttpCalls<'_, Dispatch>
where
    DispatchError: StdError + Send + Sync + 'static,
    Dispatch: RealtimeHttpDispatch<Error = DispatchError>,
{
    // ast-grep-ignore: no-one-line-functions - the SDK's public entry point is the shared session flow's only caller
    pub async fn acreate_realtime_client_secret(
        &self,
        call: ClientSecretCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<DispatchError>> {
        self.session_call(ClientSecret(&call), logging).await
    }

    // ast-grep-ignore: no-long-names, no-one-line-functions - LiteLLM's public SDK entry point keeps its name
    pub async fn acreate_realtime_transcription_session(
        &self,
        call: TranscriptionSessionCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<DispatchError>> {
        self.session_call(Transcription(&call), logging).await
    }

    pub async fn arealtime_calls(
        &self,
        call: RealtimeCallsCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<DispatchError>> {
        let model_name = body::model_or_default(call.common.model);
        let (resolved, provider_credentials) = self.route(&model_name, &call.common)?;
        let session = body::calls_session(call.session, &resolved.model)?;
        let timeout = self.timeout(call.common.timeout)?;
        let request = calls_request(
            &call,
            &resolved,
            &provider_credentials,
            session.as_ref(),
            timeout,
        );
        log_pre_call(
            logging,
            &call.common,
            &resolved,
            &provider_credentials.api_base,
            calls_params(session.as_ref()),
        )?;
        self.dispatch
            .exchange(request, logging)
            .await
            .map_err(RealtimeHttpCallsError::Dispatch)
    }

    // async_realtime_client_secret_handler and async_realtime_transcription_session_handler: prepare, log and POST one session.
    async fn session_call(
        &self,
        operation: SessionOperation<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<DispatchError>> {
        let plan = session_plan(operation)?;
        let (resolved, provider_credentials) = self.route(&plan.model_name, plan.common)?;
        log_pre_call(
            logging,
            plan.common,
            &resolved,
            &provider_credentials.api_base,
            plan.optional_params,
        )?;
        let request_data = plan.records.body(&resolved.model)?;
        let timeout = self.timeout(plan.common.timeout)?;
        let request = RealtimeHttpRequest::Session(SessionRequest {
            endpoint: plan.endpoint,
            provider: provider_credentials.config.as_ref(),
            api_base: &provider_credentials.api_base,
            api_key: &provider_credentials.api_key,
            request_data: &request_data,
            model: &resolved.model,
            extra_headers: plan.common.extra_headers,
            api_version: plan.common.api_version,
            timeout,
            client: plan.common.client,
        });
        let answer = self.dispatch.exchange(request, logging).await;
        answer.map_err(RealtimeHttpCallsError::Dispatch)
    }

    // get_llm_provider's routing and _get_realtime_http_provider_config's credentials for one model hint.
    fn route(
        &self,
        model_name: &str,
        common: &CallCommon<'_>,
    ) -> Result<(ResolvedModel, RealtimeHttpCredentials), RealtimeHttpCallsError<DispatchError>>
    {
        // Neither method forwards a caller's custom_llm_provider kwarg, so only the model decides the provider.
        let resolved = resolve(
            &ResolutionRequest {
                model: model_name,
                custom_llm_provider: None,
                api_base: common.api_base,
                api_key: common.api_key,
            },
            &self.resolution,
        )?;
        let provider_credentials =
            credentials(&resolved, self.settings, common.api_base, common.api_key);
        Ok((resolved, provider_credentials))
    }

    // `timeout or request_timeout`: zero and None select the environment's REQUEST_TIMEOUT.
    fn timeout(
        &self,
        timeout: Option<f64>,
    ) -> Result<Duration, RealtimeHttpCallsError<DispatchError>> {
        let seconds = timeout
            .filter(|seconds| *seconds != 0.0)
            .unwrap_or(self.settings.request_timeout_seconds);
        Duration::try_from_secs_f64(seconds)
            .map_err(|_invalid| RealtimeHttpCallsError::Timeout { seconds })
    }
}

// arealtime_calls' logged params: the rewritten session the call carries.
fn calls_params(session: Option<&IndexMap<String, Json>>) -> IndexMap<String, Json> {
    let mut logged = params([("realtime_calls", Json::Bool(true))]);
    logged.insert(
        String::from("session"),
        session.map_or(Json::Null, |fields| Json::Object(fields.clone())),
    );
    logged
}

// async_realtime_calls_handler's request: the caller's ephemeral key, the SDP offer and the rewritten session.
fn calls_request<'req>(
    call: &'req RealtimeCallsCall<'req>,
    resolved: &'req ResolvedModel,
    provider_credentials: &'req RealtimeHttpCredentials,
    session: Option<&'req IndexMap<String, Json>>,
    timeout: Duration,
) -> RealtimeHttpRequest<'req> {
    RealtimeHttpRequest::Calls(CallsRequest {
        provider: provider_credentials.config.as_ref(),
        api_base: &provider_credentials.api_base,
        ephemeral_key: call.openai_ephemeral_key,
        sdp: call.sdp_body,
        model: &resolved.model,
        session,
        extra_headers: call.common.extra_headers,
        api_version: call.common.api_version,
        timeout,
        client: call.common.client,
    })
}

// The endpoint's routing hint, logged raw params and records, decoded before any provider is resolved.
fn session_plan(operation: SessionOperation<'_>) -> Result<SessionPlan<'_>, RecordError> {
    Ok(match operation {
        ClientSecret(call) => {
            let session = body::optional(call.session, "session")?;
            let expires_after = body::optional(call.expires_after, "expires_after")?;
            SessionPlan {
                common: &call.common,
                model_name: body::client_secret_model(session.as_ref(), call.common.model),
                endpoint: SessionEndpoint::ClientSecrets,
                optional_params: params([
                    (
                        "expires_after",
                        call.expires_after.cloned().unwrap_or(Json::Null),
                    ),
                    ("session", call.session.cloned().unwrap_or(Json::Null)),
                ]),
                records: SessionRecords::ClientSecret {
                    session,
                    expires_after,
                },
            }
        }
        Transcription(call) => {
            let (record, model_name) =
                body::transcription(call.transcription_session, call.common.model)?;
            SessionPlan {
                common: &call.common,
                model_name,
                endpoint: SessionEndpoint::TranscriptionSessions,
                optional_params: params([(
                    "transcription_session",
                    call.transcription_session.cloned().unwrap_or(Json::Null),
                )]),
                records: SessionRecords::Transcription(record),
            }
        }
    })
}
