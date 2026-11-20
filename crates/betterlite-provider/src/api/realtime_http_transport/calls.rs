use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::realtime_contracts::config::{BaseRealtimeHttpConfig, RealtimeUrl};
use betterlite_core::api::realtime_contracts::error::HeaderMap;
use betterlite_core::api::vector_store_http_write::service::LogContext;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_logging::logging_pre_post::args::AdditionalArgs;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use sha2::{Digest as _, Sha256};

use super::TransportDeps;
use super::client::select;
use super::decode::decode_realtime_sdp;
use super::error::RealtimeHttpError;
use super::send::{WireRequest, send};

// async_realtime_calls_handler's input: the ephemeral key, the SDP offer and the session the caller asked for
pub struct CallsCall<'call> {
    pub api_base: &'call str,
    pub ephemeral_key: &'call str,
    pub sdp: &'call [u8],
    pub model: Option<&'call str>,
    pub session: Option<&'call IndexMap<String, Json>>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub api_version: Option<&'call str>,
    pub timeout: Option<Duration>,
}

// The SDP exchange goes out as multipart: the offer as text/plain and the session as application/json, both unnamed
pub async fn calls_post<C: BaseRealtimeHttpConfig>(
    deps: &TransportDeps<'_, C>,
    call: &CallsCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<UpstreamResponse, RealtimeHttpError> {
    let client = select(deps.source, deps.supplied)?;
    let model = call.model.unwrap_or_default();
    let (url, mut headers) = endpoint(deps.config, call, model);
    for (name, value) in call.extra_headers.into_iter().flatten() {
        headers.insert(name.clone(), Json::Text(value.clone()));
    }
    let session = session_data(call.session, call.model);
    let sdp = decode_realtime_sdp(call.sdp, deps.limits)?;
    let form = parts(sdp, &session)?;
    let record = PreCall {
        input: Json::Text(String::from("realtime_sdp_offer")),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(url.clone()),
            headers: Some(headers.clone().into_iter().collect()),
            complete_input_dict: None,
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::from([(String::from("session"), Json::Object(session))]),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    let separator = boundary(&form);
    send(
        deps.config,
        &client,
        WireRequest {
            url: &url,
            headers: &headers,
            body: RequestBody::Multipart {
                boundary: separator,
                parts: form,
            },
            timeout: call.timeout,
        },
    )
    .await
}

fn endpoint<C: BaseRealtimeHttpConfig>(
    config: Option<&C>,
    call: &CallsCall<'_>,
    model: &str,
) -> (String, HeaderMap) {
    let Some(provider) = config else {
        let base = call.api_base.trim_end_matches('/');
        let mut headers = HeaderMap::new();
        headers.insert(
            String::from("Authorization"),
            Json::Text(format!("Bearer {}", call.ephemeral_key)),
        );
        return (format!("{base}/v1/realtime/calls"), headers);
    };
    let url = provider.get_realtime_calls_url(&RealtimeUrl {
        api_base: Some(call.api_base),
        model,
        api_version: call.api_version,
    });
    (url, provider.get_realtime_calls_headers(call.ephemeral_key))
}

// session_config or {}, with type and model inserted only where the caller left them out; an explicit null stays
pub(crate) fn session_data(
    session: Option<&IndexMap<String, Json>>,
    model: Option<&str>,
) -> IndexMap<String, Json> {
    let mut data = session.cloned().unwrap_or_default();
    if !data.contains_key("type") {
        data.insert(String::from("type"), Json::Text(String::from("realtime")));
    }
    if !data.contains_key("model")
        && let Some(name) = model
    {
        data.insert(String::from("model"), Json::Text(name.to_owned()));
    }
    data
}

fn parts(
    sdp: &str,
    session: &IndexMap<String, Json>,
) -> Result<Vec<MultipartPart>, RealtimeHttpError> {
    let encoded = serde_json::to_vec(session).map_err(RealtimeHttpError::Encode)?;
    Ok(vec![
        MultipartPart {
            name: String::from("sdp"),
            file_name: None,
            content_type: Some(String::from("text/plain")),
            data: sdp.as_bytes().to_vec(),
        },
        MultipartPart {
            name: String::from("session"),
            file_name: None,
            content_type: Some(String::from("application/json")),
            data: encoded,
        },
    ])
}

// A boundary derived from the parts, so a body carrying it would have to hold a hash of itself
fn boundary(parts: &[MultipartPart]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.name.as_bytes());
        hasher.update(part.data.as_slice());
        hasher.update(part.content_type.as_deref().unwrap_or_default());
    }
    hex::encode(hasher.finalize())
}
