use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use betterlite_azure::transcription::call::AzureTranscriptionError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_openai::transcription::duration::duration_number;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use serde_json::{Value, json};

pub(super) fn request_view(request: &OutboundRequest) -> Value {
    let present: BTreeMap<&str, &str> = ["api-key", "authorization", "accept"]
        .into_iter()
        .filter_map(|name| Some((name, request.headers.get(name)?.to_str().ok()?)))
        .collect();
    let mut parts = Vec::new();
    if let RequestBody::Multipart { parts: sent, .. } = &request.body {
        for part in sent {
            parts.push(json!({
                "name": part.name,
                "filename": part.file_name,
                "content_type": part.content_type,
                "body": BASE64.encode(&part.data),
            }));
        }
    }
    json!({
        "url": request.url.as_str(),
        "api_key": present.get("api-key"),
        "authorization": present.get("authorization"),
        "accept": present.get("accept"),
        "parts": parts,
    })
}

pub(super) fn result_view(reply: &TranscriptionReply) -> Result<Value, Box<HelperError>> {
    let (duration_recorded, duration) = match reply.duration {
        StoredDuration::Absent => (false, Value::Null),
        StoredDuration::Known(found) => (
            true,
            serde_json::to_value(duration_number(found).or_config_error()?).or_config_error()?,
        ),
    };
    Ok(json!({
        "model": reply.model,
        "provider": reply.custom_llm_provider,
        "duration_recorded": duration_recorded,
        "duration": duration,
        "text": reply.response.text,
        "additional_headers": reply.provider_headers.additional_headers,
        "original_response": reply.original_response,
    }))
}

pub(super) fn error_view(error: &AzureTranscriptionError) -> Value {
    if let AzureTranscriptionError::Status(status) = error {
        json!({
            "status": status.status,
            "type": status.error_type,
            "param": status.param,
            "code": status.code,
        })
    } else {
        json!({ "kind": "error" })
    }
}
