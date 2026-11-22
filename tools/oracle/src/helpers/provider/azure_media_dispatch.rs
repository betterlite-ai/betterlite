use std::fmt::Display;
use std::time::Duration;

use betterlite_azure::endpoints::projection::{LiteLlmParams, SessionKind};
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::image_edits::file::ImageEditFile;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::azure_media_dispatch::call::{
    AzureMediaOperation, ImageEditOperation, ImageGenerationOperation, TranscriptionOperation,
};
use betterlite_provider::azure_media_dispatch::error::AzureMediaError;
use betterlite_provider::azure_media_dispatch::test_support::{Reply, exchange};
use betterlite_provider::azure_media_dispatch::{AzureMediaResult, ImageEditOutcome};
use betterlite_provider::operation::OperationResult;
use http::HeaderMap;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::runtime::Builder;
use uuid::Uuid;

use self::case::{Input, Kind};

mod case;
mod tests;

const NOW: i64 = 1_700_000_000;
const EDIT_REPLY: &str = r#"{"created":1700000000,"data":[{"b64_json":"aW1hZ2U="}]}"#;
const IMAGE: &[u8] = b"PNGDATA";
const AUDIO: &[u8] = b"abc";
const AUDIO_FILE: &str = "a.wav";
const AUDIO_TYPE: &str = "audio/wav";
const OTHER_OUTCOME: &str = "the dispatch returned another outcome";

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .build()
            .map_err(|error| failure("RuntimeError", error))?;
        runtime.block_on(dispatch_input(input))
    }
}

async fn dispatch_input(input: &Input) -> HelperResult {
    match input.case {
        Kind::ImageGeneration => image_generation(input).await,
        Kind::ImageEdit => image_edit(input).await,
        Kind::Transcription => transcription(input).await,
    }
}

pub(super) fn failure<M>(class: &str, message: M) -> Box<HelperError>
where
    M: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

// A leaf failure the input never meant to raise; each successful row projects its own reply
fn leaf_failure(error: &AzureMediaError) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("LeafFailure"),
        status: error.status(),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn header<'value>(headers: &'value HeaderMap, name: &str) -> Option<&'value str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

async fn image_generation(input: &Input) -> HelperResult {
    let body = serde_json::to_string(&input.reply).map_err(|error| failure("Serialize", error))?;
    let operation = ImageGenerationOperation {
        model: Some(input.model.clone()),
        prompt: input.prompt.clone(),
        timeout: Some(600.0),
        optional_params: input.options.clone(),
        api_key: Some(SecretString::from(input.api_key.clone())),
        api_base: Some(input.api_base.clone()),
        api_version: Some(input.api_version.clone()),
        now_seconds: NOW,
        ..ImageGenerationOperation::default()
    };
    let outcome = exchange(
        AzureMediaOperation::ImageGeneration(Box::new(operation)),
        vec![Reply::json(200, &body)],
    )
    .await;
    match outcome.result {
        Ok(OperationResult::AzureMedia(AzureMediaResult::ImageGeneration(image))) => {
            let requests = outcome.fixture.images();
            let Some(sent) = requests.first() else {
                return Err(failure("Fixture", "the generation sends a request"));
            };
            let sent_body: Value =
                serde_json::from_slice(&sent.body).map_err(|error| failure("Decode", error))?;
            let first = image.reply.data.first();
            Ok(json!({
                "sent_url": sent.url.as_str(),
                "sent_body": sent_body,
                "api_key": header(&sent.headers, "api-key"),
                "authorization": header(&sent.headers, "authorization"),
                "created": image.reply.created,
                "b64_json": first.and_then(|datum| datum.b64_json.as_deref()),
                "image_url": first.and_then(|datum| datum.url.as_deref()),
                "images": image.reply.data.len(),
            }))
        }
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Err(leaf_failure(&error)),
    }
}

async fn image_edit(input: &Input) -> HelperResult {
    let operation = ImageEditOperation {
        model: input.model.clone(),
        prompt: Some(input.prompt.clone()),
        image: Some(vec![Some(ImageEditFile {
            content: IMAGE.to_vec(),
            name: None,
        })]),
        options: input.options.clone(),
        api_key: Some(SecretString::from(input.api_key.clone())),
        api_base: Some(input.api_base.clone()),
        api_version: Some(input.api_version.clone()),
        boundary: Uuid::from_u128(7),
        now_seconds: NOW,
        ..ImageEditOperation::default()
    };
    let outcome = exchange(
        AzureMediaOperation::ImageEdit(Box::new(operation)),
        vec![Reply::json(200, EDIT_REPLY)],
    )
    .await;
    match outcome.result {
        Ok(OperationResult::AzureMedia(AzureMediaResult::ImageEdit(edit))) => Ok(edit_view(&edit)),
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Err(leaf_failure(&error)),
    }
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

async fn transcription(input: &Input) -> HelperResult {
    let body = serde_json::to_string(&input.reply).map_err(|error| failure("Serialize", error))?;
    let operation = TranscriptionOperation {
        input: String::from(AUDIO_FILE),
        file: ProcessedAudioFile {
            file_content: AUDIO.to_vec(),
            filename: String::from(AUDIO_FILE),
            content_type: String::from(AUDIO_TYPE),
        },
        boundary: Uuid::from_u128(11),
        model: input.model.clone(),
        optional_params: IndexMap::new(),
        atranscription: false,
        api_version: Some(input.api_version.clone()),
        api_key: Some(SecretString::from(input.api_key.clone())),
        api_base: Some(input.api_base.clone()),
        azure_ad_token: None,
        max_retries: None,
        timeout: Duration::from_secs(30),
        params: LiteLlmParams::default(),
        provider: None,
        supplied: None,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
    };
    let outcome = exchange(
        AzureMediaOperation::Transcription(Box::new(operation)),
        vec![Reply::json(200, &body).with_header("content-type", "application/json")],
    )
    .await;
    match outcome.result {
        Ok(OperationResult::AzureMedia(AzureMediaResult::Transcription(reply))) => {
            let requests = outcome.fixture.outbound();
            let Some(sent) = requests.first() else {
                return Err(failure("Fixture", "the transcription sends a request"));
            };
            transcription_view(&reply, sent)
        }
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Err(leaf_failure(&error)),
    }
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
