use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;
use betterlite_openai::videos::reply::{VideoObject, VideoObjectType};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use url::Url;

use crate::api::fal::video_prepare::VideoHeaders;
use crate::api::fal::video_prepare::create::FAL_AI_PROVIDER;
use crate::api::fal::video_prepare::decode::{FalVideoReply, decode_fal_video_reply};
use crate::api::fal::video_prepare::error::FalVideoError;
use crate::api::fal::video_prepare::paths::{
    model_path_from_request_url, request_id_from_request_url,
};

use super::UpstreamReply;
use super::error::VideoResultsError;
use super::text::{error_text, terminal_result_error};
use super::transport::{ClientKind, SuppliedTransport, VideoClients, VideoTransport, header_map};

const DEFAULT_STATUS: &str = "IN_QUEUE";
const COMPLETED_STATUS: &str = "COMPLETED";
const STATUS_SUFFIX: &str = "/status";
const QUEUED_REPLY_STATUS: &str = "queued";
const IN_PROGRESS_REPLY_STATUS: &str = "in_progress";
const COMPLETED_REPLY_STATUS: &str = "completed";
const FAILED_REPLY_STATUS: &str = "failed";
const FAL_ERROR_CODE: &str = "fal_error";
const RESULT_HEADERS_NOT_FORWARDED: [&str; 3] = ["host", "content-length", "transfer-encoding"];

// the status request as fal.ai received it: the probe repeats it with only its trailing /status removed
pub struct StatusRequest {
    pub url: Url,
    pub headers: VideoHeaders,
}

pub struct StatusCall<'call> {
    pub request: StatusRequest,
    pub reply: &'call UpstreamReply,
    pub provider: Option<&'call str>,
}

pub async fn status_response<T>(
    clients: &VideoClients<'_, T>,
    kind: ClientKind,
    supplied: Option<&SuppliedTransport<'_, T>>,
    call: &StatusCall<'_>,
) -> Result<VideoObject, VideoResultsError>
where
    T: VideoTransport + Sync,
{
    let reply = decode_fal_video_reply(call.reply.status, &call.reply.headers, &call.reply.body)?;
    let client = supplied
        .filter(|client| client.kind == kind)
        .map_or_else(|| clients.for_kind(kind), |client| client.transport);
    let result_error = probe_result_error(client, &call.request, &reply).await?;
    status_video_object(&reply, &call.request.url, call.provider, result_error)
}

async fn probe_result_error<T>(
    client: &T,
    request: &StatusRequest,
    reply: &FalVideoReply,
) -> Result<Option<String>, VideoResultsError>
where
    T: VideoTransport + Sync,
{
    let Some((url, headers)) = result_request(reply, request) else {
        return Ok(None);
    };
    let probe_url = Url::parse(&url).map_err(VideoResultsError::ProbeUrl)?;
    let response = match client.get(probe_url, header_map(&headers)?).await {
        Ok(response) => response,
        // LiteLLM's probe catches only httpx.TransportError, so these failures keep a completed job completed
        Err(
            TransportError::Connect(_)
            | TransportError::ConnectTimeout(_)
            | TransportError::Deadline(_)
            | TransportError::Request(_)
            | TransportError::Body(_)
            | TransportError::RedirectScheme { .. }
            | TransportError::Mock(_),
        ) => return Ok(None),
        // a redirect loop, a refused request or an answer over the transport's limit surfaces instead
        Err(source) => return Err(VideoResultsError::ProbeTransport { source }),
    };
    Ok(terminal_result_error(response.status, &response.body))
}

#[must_use]
pub fn result_request(
    reply: &FalVideoReply,
    request: &StatusRequest,
) -> Option<(String, VideoHeaders)> {
    let status = reply.status.as_deref().unwrap_or(DEFAULT_STATUS);
    if status != COMPLETED_STATUS {
        return None;
    }
    let raw = request.url.as_str();
    let result_url = raw.strip_suffix(STATUS_SUFFIX).unwrap_or(raw).to_owned();
    let headers = request
        .headers
        .iter()
        .filter(|(name, _)| {
            !RESULT_HEADERS_NOT_FORWARDED
                .iter()
                .any(|blocked| name.eq_ignore_ascii_case(blocked))
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Some((result_url, headers))
}

pub fn status_video_object(
    reply: &FalVideoReply,
    url: &Url,
    provider: Option<&str>,
    result_error: Option<String>,
) -> Result<VideoObject, VideoResultsError> {
    let path = decoded_path(url);
    let raw_status = reply.status.as_deref().unwrap_or(DEFAULT_STATUS);
    let error = result_error.or_else(|| error_text(reply));
    let message = error.as_deref().filter(|message| !message.is_empty());
    let provider_name = provider
        .filter(|candidate| !candidate.is_empty())
        .unwrap_or(FAL_AI_PROVIDER);
    let model_path = model_path_from_request_url(&path);
    let request_id = reply
        .request_id
        .as_deref()
        .filter(|request_id| !request_id.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| request_id_from_request_url(&path))
        .unwrap_or_default();
    let wire = StatusWire {
        id: encode_video_id_with_provider(&request_id, provider_name, model_path.as_deref()),
        object: VideoObjectType::Video,
        status: match message {
            Some(_) => FAILED_REPLY_STATUS,
            None => mapped_status(raw_status),
        },
        created_at: 0,
        model: model_path.as_deref(),
        error: message.map(|text| FalErrorWire {
            code: FAL_ERROR_CODE,
            message: text,
        }),
    };
    let value = serde_json::to_value(wire).map_err(FalVideoError::VideoObjectEncode)?;
    let video =
        serde_json::from_value::<VideoObject>(value).map_err(FalVideoError::VideoObjectDecode)?;
    Ok(video)
}

// httpx's URL.path is the percent-decoded path, and both the model and the request id are read from it
fn decoded_path(url: &Url) -> String {
    percent_decode_str(url.path())
        .decode_utf8_lossy()
        .into_owned()
}

fn mapped_status(status: &str) -> &'static str {
    match status {
        COMPLETED_STATUS => COMPLETED_REPLY_STATUS,
        "IN_PROGRESS" => IN_PROGRESS_REPLY_STATUS,
        _ => QUEUED_REPLY_STATUS,
    }
}

#[derive(Serialize)]
struct StatusWire<'wire> {
    id: String,
    object: VideoObjectType,
    status: &'static str,
    created_at: i64,
    model: Option<&'wire str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<FalErrorWire<'wire>>,
}

#[derive(Serialize)]
struct FalErrorWire<'wire> {
    code: &'static str,
    message: &'wire str,
}
