use std::collections::BTreeMap;

use betterlite_core::api::fal::video_prepare::VideoHeaders;
use betterlite_core::api::fal::video_results::UpstreamReply;
use betterlite_core::api::fal::video_results::content::content_response;
use betterlite_core::api::fal::video_results::error::VideoResultsError;
use betterlite_core::api::fal::video_results::status::{
    StatusCall, StatusRequest, status_response,
};
use betterlite_core::api::fal::video_results::transport::{ClientKind, VideoClients};
use betterlite_core::api::fal::video_results::unsupported::ALL;
use betterlite_openai::videos::reply::VideoObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};
use url::Url;

use super::input::{ContentCase, StatusCase};
use super::script::ScriptedTransport;

type RecordedRequest = (String, BTreeMap<String, String>);

pub(crate) fn status_report(cases: &[StatusCase]) -> HelperResult {
    let outcomes: Vec<Value> = cases
        .iter()
        .map(status_case)
        .collect::<Result<_, Box<HelperError>>>()?;
    Ok(json!({"cases": outcomes}))
}

fn status_case(case: &StatusCase) -> Result<Value, Box<HelperError>> {
    let sync_outcome = status_outcome(case, ClientKind::Sync)?;
    let asynchronous_outcome = status_outcome(case, ClientKind::Asynchronous)?;
    Ok(json!({"sync": sync_outcome, "async": asynchronous_outcome}))
}

fn status_outcome(case: &StatusCase, kind: ClientKind) -> Result<Value, Box<HelperError>> {
    // one outcome issues one request, so both kinds answer from the same scripted reply
    let transport = ScriptedTransport::new(case.probe_status, case.probe_body.as_bytes());
    let clients = VideoClients {
        sync: &transport,
        asynchronous: &transport,
    };
    let request = StatusRequest {
        url: Url::parse(&case.status_url).or_value_error()?,
        headers: case.headers.clone(),
    };
    let reply = UpstreamReply {
        status: 200,
        headers: VideoHeaders::new(),
        body: case.body.as_bytes().to_vec(),
    };
    let call = StatusCall {
        request,
        reply: &reply,
        provider: case.provider.as_deref(),
    };
    let video = runtime()?
        .block_on(status_response(&clients, kind, None, &call))
        .or_value_error()?;
    Ok(json!({
        "video": video_fields(&video),
        "requests": request_values(&transport.requests()),
    }))
}

pub(crate) fn content_report(cases: &[ContentCase]) -> HelperResult {
    let outcomes: Vec<Value> = cases
        .iter()
        .map(content_case)
        .collect::<Result<_, Box<HelperError>>>()?;
    Ok(json!({"cases": outcomes}))
}

fn content_case(case: &ContentCase) -> Result<Value, Box<HelperError>> {
    let sync_outcome = content_outcome(case, ClientKind::Sync)?;
    let asynchronous_outcome = content_outcome(case, ClientKind::Asynchronous)?;
    Ok(json!({"sync": sync_outcome, "async": asynchronous_outcome}))
}

fn content_outcome(case: &ContentCase, kind: ClientKind) -> Result<Value, Box<HelperError>> {
    let transport = ScriptedTransport::new(case.artifact_status, case.artifact_body.as_bytes());
    let outcome = content_bytes(&transport, kind, &content_reply(case))?;
    let requests = request_values(&transport.requests());
    Ok(content_value(&outcome, &requests))
}

fn content_reply(case: &ContentCase) -> UpstreamReply {
    UpstreamReply {
        status: case.status,
        headers: case.response_headers.clone(),
        body: case.body.as_bytes().to_vec(),
    }
}

fn content_bytes(
    transport: &ScriptedTransport,
    kind: ClientKind,
    reply: &UpstreamReply,
) -> Result<Result<Vec<u8>, VideoResultsError>, Box<HelperError>> {
    // one outcome issues one request, so both kinds answer from the same scripted reply
    let clients = VideoClients {
        sync: transport,
        asynchronous: transport,
    };
    Ok(runtime()?.block_on(content_response(&clients, kind, reply)))
}

fn content_value(outcome: &Result<Vec<u8>, VideoResultsError>, requests: &[Value]) -> Value {
    match outcome {
        Ok(body) => json!({"body": str::from_utf8(body).ok(), "error": null, "requests": requests}),
        Err(error) => json!({"body": null, "error": error_fields(error), "requests": requests}),
    }
}

fn error_fields(error: &VideoResultsError) -> Value {
    match error {
        VideoResultsError::Upstream(exception) => json!({
            "kind": "fal",
            "status": exception.status_code,
            "message": exception.message,
        }),
        VideoResultsError::ArtifactStatus { status } => json!({
            "kind": "artifact",
            "status": status,
            "message": null,
        }),
        VideoResultsError::ArtifactTransport(_) => json!({
            "kind": "transport",
            "status": null,
            "message": null,
        }),
        VideoResultsError::MissingVideoUrl | VideoResultsError::MissingVideoUrlDetail { .. } => {
            json!({
                "kind": "value",
                "status": null,
                "message": error.to_string(),
            })
        }
        VideoResultsError::Reply(_)
        | VideoResultsError::ProbeUrl(_)
        | VideoResultsError::ProbeTransport { .. }
        | VideoResultsError::ArtifactUrl(_)
        | VideoResultsError::HeaderName { .. }
        | VideoResultsError::HeaderValue { .. }
        | VideoResultsError::Unsupported(_) => json!({
            "kind": null,
            "status": null,
            "message": null,
        }),
    }
}

pub(crate) fn unsupported_report() -> Value {
    let operations: Vec<Value> = ALL
        .iter()
        .map(|operation| json!({"name": operation.name(), "message": operation.to_string()}))
        .collect();
    json!({"operations": operations})
}

fn runtime() -> Result<Runtime, Box<HelperError>> {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()
}

fn request_values(requests: &[RecordedRequest]) -> Vec<Value> {
    requests
        .iter()
        .map(|(url, headers)| json!({"url": url, "headers": headers}))
        .collect()
}

fn video_fields(video: &VideoObject) -> Value {
    json!({
        "id": video.id,
        "object": video.object,
        "status": video.status,
        "created_at": video.created_at,
        "completed_at": video.completed_at,
        "expires_at": video.expires_at,
        "error": video.error,
        "progress": video.progress,
        "remixed_from_video_id": video.remixed_from_video_id,
        "seconds": video.seconds,
        "size": video.size,
        "model": video.model,
        "usage": video.usage,
    })
}
