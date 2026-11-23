use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::azure_media_dispatch::AzureMediaRequest;
use betterlite_provider::azure_media_dispatch::call::AzureMediaOperation;
use betterlite_provider::azure_media_dispatch::error::AzureMediaError;
use betterlite_provider::azure_media_dispatch::test_support::exchange::exchange_request_with;
use betterlite_provider::azure_media_dispatch::test_support::{Outcome, Reply};
use betterlite_provider::azure_media_dispatch::{AzureMediaResult, ImageEditOutcome};
use betterlite_provider::operation::{OperationRequest, OperationResult};
use http::HeaderMap;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use self::case::{Input, Kind};
use crate::openai_media_dispatch::{OTHER_OUTCOME, failure};

mod case;
mod edit;
mod generation;
mod speech;
mod tests;
mod transcription;

const NOW: i64 = 1_700_000_000;
const EDIT_REPLY: &str = r#"{"created":1700000000,"data":[{"b64_json":"aW1hZ2U="}]}"#;
const IMAGE: &[u8] = b"PNGDATA";
const AUDIO: &[u8] = b"abc";
const AUDIO_FILE: &str = "a.wav";
const AUDIO_TYPE: &str = "audio/wav";

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error))?
            .block_on(dispatch_input(input))
    }
}

// One dispatch per case: each arm builds the call the caller resolved, runs it and projects the reply
async fn dispatch_input(input: &Input) -> HelperResult {
    match input.case {
        Kind::ImageGeneration => generation::image_generation(input).await,
        Kind::ImageEdit => edit::image_edit(input).await,
        Kind::OpenAiSpeech => speech::run(input, speech::Arm::OpenAi).await,
        Kind::NativeSsml => speech::run(input, speech::Arm::Native).await,
        Kind::Transcription => transcription::transcription(input).await,
    }
}

// Every media arm dispatches the same unit request with an untouched speech logger; only the logging fixture can fail
pub(super) async fn exchange(
    operation: AzureMediaOperation,
    replies: Vec<Reply>,
) -> Result<Outcome, Box<HelperError>> {
    exchange_request_with(
        OperationRequest::AzureMedia(AzureMediaRequest),
        operation,
        replies,
        |_logging| {},
    )
    .await
    .map_err(|error| failure("RuntimeError", error))
}

// The case's reply as the wire body the JSON legs decode
fn json_reply(input: &Input) -> Result<Reply, Box<HelperError>> {
    serde_json::to_string(&input.reply)
        .map_err(|error| failure("Serialize", error))
        .map(|body| Reply::json(200, &body))
}

// A leaf failure the input never meant to raise; each successful row projects its own reply
fn leaf_failure(error: &AzureMediaError) -> Box<HelperError> {
    let mut raised = failure("LeafFailure", error);
    raised.status = error.status();
    raised
}

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

// The wire request the generation leaf sent and the reply fields both sides declare
fn generation_result(outcome: &Outcome) -> HelperResult {
    let Ok(OperationResult::AzureMedia(AzureMediaResult::ImageGeneration(image))) = &outcome.result
    else {
        return outcome_error(&outcome.result);
    };
    let sent = first_request(outcome.fixture.images())?;
    let body: Value =
        serde_json::from_slice(&sent.body).map_err(|error| failure("Decode", error))?;
    let first = image.reply.data.first();
    Ok(json!({
        "sent_url": sent.url.as_str(),
        "sent_body": body,
        "api_key": header(&sent.headers, "api-key"),
        "authorization": header(&sent.headers, "authorization"),
        "created": image.reply.created,
        "b64_json": first.and_then(|datum| datum.b64_json.as_deref()),
        "image_url": first.and_then(|datum| datum.url.as_deref()),
        "images": image.reply.data.len(),
    }))
}

// The form data and auth headers the resolved edit carries, without the random multipart boundary
fn edit_view(edit: &ImageEditOutcome) -> Value {
    let mut fields: Vec<(&str, &Json)> = edit
        .data
        .iter()
        .map(|(name, value)| (name.as_str(), value))
        .collect();
    fields.sort_by(|left, right| left.0.cmp(right.0));
    json!({
        "url": edit.url,
        "fields": fields,
        "auth": edit.headers,
    })
}

// A failed dispatch raises its leaf error; a successful dispatch of another shape is a broken fixture
fn outcome_error(result: &Result<OperationResult, AzureMediaError>) -> HelperResult {
    match result {
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Err(leaf_failure(error)),
    }
}

fn first_request<T>(requests: Vec<T>) -> Result<T, Box<HelperError>> {
    let Some(request) = requests.into_iter().next() else {
        return Err(failure("Fixture", "the dispatch sends a request"));
    };
    Ok(request)
}

// The wire request and reply projections the Python helper compares
fn transcription_view(reply: &TranscriptionReply, sent: &OutboundRequest) -> HelperResult {
    let RequestBody::Multipart { parts, .. } = &sent.body else {
        return Err(failure("Fixture", "the transcription request is multipart"));
    };
    let entries = audio_parts(parts);
    Ok(json!({
        "url": sent.url.as_str(),
        "api_key": header(&sent.headers, "api-key"),
        "accept": header(&sent.headers, "accept"),
        "parts": entries,
        "text": reply.response.text.as_deref(),
        "model": reply.model.as_deref(),
        "provider": reply.custom_llm_provider,
        "duration_recorded": !matches!(reply.duration, StoredDuration::Absent),
    }))
}

fn audio_parts(parts: &[MultipartPart]) -> Vec<Value> {
    let mut ordered: Vec<&MultipartPart> = parts.iter().collect();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    ordered
        .into_iter()
        .map(|part| {
            json!({
                "name": part.name,
                "filename": part.file_name,
                "content_type": part.content_type,
                "body": String::from_utf8_lossy(&part.data),
            })
        })
        .collect()
}
