use std::error::Error as StdError;
use std::time::Duration;

use betterlite_config::api::realtime_http_calls::RealtimeHttpCallSettings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use indexmap::IndexMap;
use secrecy::SecretString;

pub mod body;
pub mod call;
pub mod dispatch;
pub mod error;
pub mod logging;
pub mod provider;

#[path = "realtime_http_calls/tests.rs"]
mod tests;

use self::call::{ClientSecretCall, RealtimeCallsCall, TranscriptionSessionCall};
use self::dispatch::{
    CallsRequest, RealtimeHttpDispatch, RealtimeHttpRequest, SessionEndpoint, SessionRequest,
};
use self::error::RealtimeHttpCallsError;
use self::logging::{PreCall, log_pre_call};
use self::provider::credentials;

// The two routing defaults the SDK applies before get_llm_provider.
pub const DEFAULT_REALTIME_MODEL: &str = "gpt-4o-realtime-preview";
pub const DEFAULT_TRANSCRIPTION_MODEL: &str = "gpt-realtime-whisper";

// The three decorated SDK methods over the deps a caller supplies once: the transport, the loaded settings and resolution's inputs.
pub struct RealtimeHttpCalls<'deps, Dispatch: RealtimeHttpDispatch> {
    pub dispatch: &'deps Dispatch,
    pub settings: &'deps RealtimeHttpCallSettings,
    pub resolution: ResolutionContext<'deps>,
}

impl<DispatchError, Dispatch> RealtimeHttpCalls<'_, Dispatch>
where
    DispatchError: StdError + Send + Sync + 'static,
    Dispatch: RealtimeHttpDispatch<Error = DispatchError>,
{
    pub async fn acreate_realtime_client_secret(
        &self,
        call: ClientSecretCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<DispatchError>> {
        let session = body::session(call.session)?;
        let expires_after = body::expires_after(call.expires_after)?;
        let model_name = body::client_secret_model(session.as_ref(), call.model);
        let resolved = self.resolve_model(&model_name, call.api_base, call.api_key)?;
        let provider_credentials =
            credentials(&resolved, self.settings, call.api_base, call.api_key);
        log_pre_call(
            logging,
            PreCall {
                kwargs: call.kwargs,
                model: &resolved.model,
                provider: resolved.provider.slug(),
                api_base: &provider_credentials.api_base,
                optional_params: client_secret_optional_params(call.session, call.expires_after),
            },
        )?;
        let request_data =
            body::client_secret(session.as_ref(), expires_after.as_ref(), &resolved.model)?;
        let timeout = self.timeout(call.timeout)?;
        let request = RealtimeHttpRequest::Session(SessionRequest {
            endpoint: SessionEndpoint::ClientSecrets,
            provider: provider_credentials.config.as_ref(),
            api_base: &provider_credentials.api_base,
            api_key: &provider_credentials.api_key,
            request_data: &request_data,
            model: &resolved.model,
            extra_headers: call.extra_headers,
            api_version: call.api_version,
            timeout,
            client: call.client,
        });
        self.post(request, logging).await
    }

    pub async fn acreate_realtime_transcription_session(
        &self,
        call: TranscriptionSessionCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<DispatchError>> {
        let (request, model_name) = body::transcription(call.transcription_session, call.model)?;
        let resolved = self.resolve_model(&model_name, call.api_base, call.api_key)?;
        let provider_credentials =
            credentials(&resolved, self.settings, call.api_base, call.api_key);
        log_pre_call(
            logging,
            PreCall {
                kwargs: call.kwargs,
                model: &resolved.model,
                provider: resolved.provider.slug(),
                api_base: &provider_credentials.api_base,
                optional_params: transcription_optional_params(call.transcription_session),
            },
        )?;
        let request_data = body::transcription_body(&request, &resolved.model)?;
        let timeout = self.timeout(call.timeout)?;
        let request = RealtimeHttpRequest::Session(SessionRequest {
            endpoint: SessionEndpoint::TranscriptionSessions,
            provider: provider_credentials.config.as_ref(),
            api_base: &provider_credentials.api_base,
            api_key: &provider_credentials.api_key,
            request_data: &request_data,
            model: &resolved.model,
            extra_headers: call.extra_headers,
            api_version: call.api_version,
            timeout,
            client: call.client,
        });
        self.post(request, logging).await
    }

    pub async fn arealtime_calls(
        &self,
        call: RealtimeCallsCall<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<DispatchError>> {
        let model_name = body::calls_model(call.model);
        let resolved = self.resolve_model(&model_name, call.api_base, call.api_key)?;
        let provider_credentials =
            credentials(&resolved, self.settings, call.api_base, call.api_key);
        let session = body::calls_session(call.session, &resolved.model)?;
        log_pre_call(
            logging,
            PreCall {
                kwargs: call.kwargs,
                model: &resolved.model,
                provider: resolved.provider.slug(),
                api_base: &provider_credentials.api_base,
                optional_params: calls_optional_params(session.as_ref()),
            },
        )?;
        let timeout = self.timeout(call.timeout)?;
        let request = RealtimeHttpRequest::Calls(CallsRequest {
            provider: provider_credentials.config.as_ref(),
            api_base: &provider_credentials.api_base,
            ephemeral_key: call.openai_ephemeral_key,
            sdp: call.sdp_body,
            model: &resolved.model,
            session: session.as_ref(),
            extra_headers: call.extra_headers,
            api_version: call.api_version,
            timeout,
            client: call.client,
        });
        self.post(request, logging).await
    }

    fn resolve_model(
        &self,
        model: &str,
        api_base: Option<&str>,
        api_key: Option<&SecretString>,
    ) -> Result<ResolvedModel, ResolutionError> {
        // Neither method forwards a caller's custom_llm_provider kwarg, so only the model decides the provider.
        resolve(
            &ResolutionRequest {
                model,
                custom_llm_provider: None,
                api_base,
                api_key,
            },
            &self.resolution,
        )
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

    // The wrapper returns the handler's response untouched and re-raises its error.
    async fn post(
        &self,
        request: RealtimeHttpRequest<'_>,
        logging: &mut LoggingState,
    ) -> Result<UpstreamResponse, RealtimeHttpCallsError<DispatchError>> {
        self.dispatch
            .exchange(request, logging)
            .await
            .map_err(RealtimeHttpCallsError::Dispatch)
    }
}

// update_from_kwargs(optional_params={"expires_after": expires_after, "session": session}) before any rewrite.
fn client_secret_optional_params(
    session: Option<&Json>,
    expires_after: Option<&Json>,
) -> IndexMap<String, Json> {
    IndexMap::from([
        (
            String::from("expires_after"),
            expires_after.cloned().unwrap_or(Json::Null),
        ),
        (
            String::from("session"),
            session.cloned().unwrap_or(Json::Null),
        ),
    ])
}

// update_from_kwargs(optional_params={"transcription_session": transcription_session}) with the caller's mapping.
fn transcription_optional_params(transcription_session: Option<&Json>) -> IndexMap<String, Json> {
    IndexMap::from([(
        String::from("transcription_session"),
        transcription_session.cloned().unwrap_or(Json::Null),
    )])
}

// update_from_kwargs(optional_params={"realtime_calls": True, "session": session}) after the model rewrite.
fn calls_optional_params(session: Option<&IndexMap<String, Json>>) -> IndexMap<String, Json> {
    IndexMap::from([
        (String::from("realtime_calls"), Json::Bool(true)),
        (
            String::from("session"),
            session.map_or(Json::Null, |session| Json::Object(session.clone())),
        ),
    ])
}
