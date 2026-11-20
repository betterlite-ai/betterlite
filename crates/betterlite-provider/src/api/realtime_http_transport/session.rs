use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::realtime_contracts::config::{
    BaseRealtimeHttpConfig, Environment, RealtimeUrl,
};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::TransportDeps;
use super::client::select;
use super::error::RealtimeHttpError;
use super::send::{WireRequest, send};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEndpoint {
    ClientSecrets,
    TranscriptionSessions,
}

impl SessionEndpoint {
    const fn path(self) -> &'static str {
        match self {
            Self::ClientSecrets => "client_secrets",
            Self::TranscriptionSessions => "transcription_sessions",
        }
    }
}

// The two session handlers' shared input: which endpoint, the call credentials and where the body goes
pub struct SessionCall<'call> {
    pub endpoint: SessionEndpoint,
    pub api_base: &'call str,
    pub api_key: &'call str,
    pub request_data: &'call IndexMap<String, Json>,
    pub model: Option<&'call str>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub api_version: Option<&'call str>,
    pub timeout: Option<Duration>,
}

// _async_realtime_session_post: the config names the URL and the validated headers, or the OpenAI-style fallback does
pub async fn session_post<C: BaseRealtimeHttpConfig>(
    deps: &TransportDeps<'_, C>,
    call: &SessionCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<UpstreamResponse, RealtimeHttpError> {
    let client = select(deps.source, deps.supplied)?;
    let model = call.model.unwrap_or_default();
    let (url, mut headers) = endpoint(deps.config, call, model)?;
    for (name, value) in call.extra_headers.into_iter().flatten() {
        headers.insert(name.clone(), Json::Text(value.clone()));
    }
    let record = PreCall {
        input: Json::Object(call.request_data.clone()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(url.clone()),
            headers: Some(headers.clone().into_iter().collect()),
            complete_input_dict: Some(RawBody::Object(call.request_data.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    let body = serde_json::to_vec(call.request_data).map_err(RealtimeHttpError::Encode)?;
    send(
        deps.config,
        &client,
        WireRequest {
            url: &url,
            headers: &headers,
            body: RequestBody::Json(body),
            timeout: call.timeout,
        },
    )
    .await
}

fn endpoint<C: BaseRealtimeHttpConfig>(
    config: Option<&C>,
    call: &SessionCall<'_>,
    model: &str,
) -> Result<(String, HeaderMap), RealtimeHttpError> {
    let Some(provider) = config else {
        let base = call.api_base.trim_end_matches('/');
        let mut headers = HeaderMap::new();
        headers.insert(
            String::from("Authorization"),
            Json::Text(format!("Bearer {}", call.api_key)),
        );
        headers.insert(
            String::from("Content-Type"),
            Json::Text(String::from("application/json")),
        );
        return Ok((
            format!("{base}/v1/realtime/{}", call.endpoint.path()),
            headers,
        ));
    };
    let empty = HeaderMap::new();
    let headers = provider
        .validate_environment(&Environment {
            headers: &empty,
            model,
            api_key: Some(call.api_key),
        })
        .map_err(|source| RealtimeHttpError::Config(Box::new(source)))?;
    let url = match call.endpoint {
        SessionEndpoint::ClientSecrets => {
            provider.get_complete_url(Some(call.api_base), model, call.api_version)
        }
        SessionEndpoint::TranscriptionSessions => {
            provider.get_transcription_session_url(&RealtimeUrl {
                api_base: Some(call.api_base),
                model,
                api_version: call.api_version,
            })
        }
    };
    Ok((url, headers))
}
