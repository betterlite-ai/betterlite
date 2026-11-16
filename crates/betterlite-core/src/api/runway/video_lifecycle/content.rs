use std::str;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use http::{HeaderMap, Method};
use url::Url;

use super::artifact::ArtifactTransport;
use super::error::VideoLifecycleError;
use super::{LifecycleRequest, task_request};
use crate::api::api_boundaries::ApiLimits;
use crate::api::runway::tasks::record::{RunwayTask, TaskOutput, TaskOutputs, decode_runway_task};
use crate::api::runway::video_create::error::get_error_class;

const STILL_PROCESSING: [&str; 3] = ["PENDING", "RUNNING", "THROTTLED"];
const FAILED_STATUS: &str = "FAILED";
const UNKNOWN_FAILURE: &str = "Unknown error";

// transform_video_content_request: the task lookup ignores the requested variant, since RunwayML has one asset
pub fn content_request(
    video_id: &str,
    api_base: Option<&str>,
    _variant: Option<&str>,
) -> Result<LifecycleRequest, VideoLifecycleError> {
    task_request(video_id, api_base, "")
}

// the extraction reads the output before the status, and only the first array element counts
fn output_url(output: Option<&TaskOutputs>) -> Result<Option<&str>, VideoLifecycleError> {
    match output {
        None => Ok(None),
        Some(TaskOutputs::Text(text)) => Ok((!text.is_empty()).then_some(text.as_str())),
        Some(TaskOutputs::Members(members)) => match members.first() {
            None => Ok(None),
            Some(TaskOutput::Url(url)) => Ok((!url.is_empty()).then_some(url.as_str())),
            Some(TaskOutput::Fields(_) | TaskOutput::Unsupported(_)) => {
                Err(VideoLifecycleError::OutputNotText)
            }
        },
        Some(TaskOutputs::NotAList(_)) => Err(VideoLifecycleError::OutputNotText),
    }
}

// response_data.get("failure", "Unknown error"): a null takes that default, and any other non-text value is refused
fn failure_reason(task: &RunwayTask) -> Result<String, VideoLifecycleError> {
    match task.failure.value() {
        None | Some(Json::Null) => Ok(UNKNOWN_FAILURE.to_owned()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(VideoLifecycleError::FailureNotText)
        }
    }
}

// the status comparison is exact and case sensitive here, unlike the retrieval status mapping
fn selected_url(task: &RunwayTask) -> Result<String, VideoLifecycleError> {
    if let Some(url) = output_url(task.output.as_ref())? {
        return Ok(url.to_owned());
    }
    match task.status.as_deref() {
        Some(status) if STILL_PROCESSING.contains(&status) => {
            Err(VideoLifecycleError::StillProcessing {
                status: status.to_owned(),
            })
        }
        Some(FAILED_STATUS) => Err(VideoLifecycleError::GenerationFailed {
            reason: failure_reason(task)?,
        }),
        _ => Err(VideoLifecycleError::MissingUrl),
    }
}

fn error_headers(headers: &HeaderMap) -> Headers {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let text = value.to_str().ok()?;
            Some((name.as_str().to_owned(), text.to_owned()))
        })
        .collect()
}

// e.response.text in LiteLLM; bytes that are not UTF-8 keep the valid prefix
fn error_text(body: &[u8]) -> String {
    match str::from_utf8(body) {
        Ok(text) => text.to_owned(),
        Err(invalid) => {
            let valid = body.get(..invalid.valid_up_to()).unwrap_or_default();
            str::from_utf8(valid).map_or_else(|_| String::new(), str::to_owned)
        }
    }
}

fn artifact_failure(response: &UpstreamResponse) -> VideoLifecycleError {
    VideoLifecycleError::ArtifactStatus(get_error_class(
        &error_text(&response.body),
        response.status,
        &error_headers(&response.headers),
    ))
}

// transform_video_content_response: extract the URL, fetch it without the provider headers, and raise on failure
pub async fn content_response<T: ArtifactTransport>(
    body: &[u8],
    transport: &T,
    limits: ApiLimits,
    deadline: Duration,
) -> Result<Vec<u8>, VideoLifecycleError> {
    let task = decode_runway_task(body)?;
    let url = selected_url(&task)?;
    let parsed = Url::parse(&url).map_err(VideoLifecycleError::ArtifactUrl)?;
    let request = OutboundRequest {
        method: Method::GET,
        url: parsed,
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Empty,
        deadline,
    };
    let response = transport.fetch(request, limits.max_json_bytes).await?;
    if !(200..300).contains(&response.status) {
        return Err(artifact_failure(&response));
    }
    Ok(response.body)
}
