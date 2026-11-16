use std::time::Duration;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::speech::client::SpeechConnection;
use betterlite_openai::speech::error::SpeechError;
use betterlite_openai::speech::request::SpeechOptions;
use betterlite_openai::transcription::client::TranscriptionConnection;
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_openai::transcription::error::TranscriptionError;
use betterlite_openai::transcription::reply::StoredDuration;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_media_dispatch::call::{SpeechOperation, TranscriptionOperation};
use betterlite_provider::openai_media_dispatch::error::OpenAiMediaError;
use betterlite_provider::openai_media_dispatch::execute::dispatch;
use betterlite_provider::openai_media_dispatch::{OpenAiMediaRequest, OpenAiMediaResult};
use betterlite_provider::operation::{OperationRequest, OperationResult};
use http::HeaderMap;
use http::HeaderValue;
use http::header::CONTENT_TYPE;
use secrecy::SecretString;
use serde_json::Value;
use uuid::Uuid;

use super::fixtures::{self, Fixture, Recorder};
use super::views::text;
use super::{OTHER_OUTCOME, SpeechCase, TranscriptionCase, failure, leaf_failure};

pub(super) async fn speech(case: &SpeechCase) -> HelperResult {
    let operation = speech_operation(case);
    let mut headers = HeaderMap::new();
    if let Some(content_type) = &case.content_type {
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::try_from(content_type.as_str())
                .map_err(|error| failure("Fixture", error))?,
        );
    }
    let fixture = Fixture::with_headers(case.status, case.body.as_bytes(), headers);
    let mut logger = Recorder;
    let result = dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::Speech(Box::new(operation)),
        &fixture,
        &mut logger,
    )
    .await;
    let sent = fixtures::only_request(&fixture)?;
    let request = speech_request(&sent, &case.options)?;
    speech_outcome(result, request).await
}

fn speech_operation(case: &SpeechCase) -> SpeechOperation {
    SpeechOperation {
        model: case.model.clone(),
        input: case.input.clone(),
        voice: case.voice.clone(),
        options: case.options.clone(),
        connection: SpeechConnection {
            api_key: Some(SecretString::from("sk-speech")),
            api_base: Some(case.api_base.clone()),
            timeout: Duration::from_secs(60),
            max_retries: 0,
            organization: None,
            project: None,
            environment_organization: None,
            environment_project: None,
            client: None,
            shared_session: None,
        },
        aspeech: false,
    }
}

async fn speech_outcome(
    result: Result<OperationResult, OpenAiMediaError>,
    request: Value,
) -> HelperResult {
    match result {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::Speech(response))) => {
            let status = response.status;
            let content_type = response.content_type.clone();
            let body = response
                .content(1_048_576)
                .await
                .map_err(|error| failure("SpeechBody", error))?;
            Ok(serde_json::json!({
                "request": request,
                "status": status,
                "content_type": content_type,
                "body": text(&body)?,
                "error": Value::Null,
            }))
        }
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(OpenAiMediaError::Speech(SpeechError::Status(raised))) => Ok(serde_json::json!({
            "request": request,
            "status": Value::Null,
            "content_type": Value::Null,
            "body": Value::Null,
            "error": {
                "status": raised.status,
                "type": raised.error_type.clone(),
                "param": raised.param.clone(),
                "code": raised.code.clone(),
            },
        })),
        Err(error) => Err(leaf_failure(&error)),
    }
}

pub(super) async fn transcription(case: &TranscriptionCase) -> HelperResult {
    let operation = transcription_operation(case)?;
    let fixture = Fixture::new(case.status, case.reply.as_bytes());
    let mut logger = Recorder;
    let result = dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::Transcription(Box::new(operation)),
        &fixture,
        &mut logger,
    )
    .await;
    let sent = fixtures::only_request(&fixture)?;
    let request = transcription_request(&sent)?;
    transcription_outcome(result, request)
}

fn transcription_operation(
    case: &TranscriptionCase,
) -> Result<TranscriptionOperation, Box<HelperError>> {
    let family = match case.family.as_str() {
        "whisper" => TranscriptionFamily::Whisper,
        "gpt" => TranscriptionFamily::Gpt,
        other => {
            return Err(failure(
                "Input",
                format!("unknown transcription family {other}"),
            ));
        }
    };
    Ok(TranscriptionOperation {
        input: String::from("a.wav"),
        file: ProcessedAudioFile {
            file_content: b"abc".to_vec(),
            filename: String::from("a.wav"),
            content_type: String::from("audio/wav"),
        },
        connection: TranscriptionConnection {
            api_key: Some(SecretString::from("sk-transcription")),
            environment_api_key: None,
            api_base: Some(case.api_base.clone()),
            timeout: Duration::from_secs(30),
            max_retries: 2,
            shared_session: None,
        },
        boundary: Uuid::nil(),
        model: case.model.clone(),
        family: Some(family),
        optional_params: case.options.clone(),
        atranscription: false,
    })
}

fn transcription_request(request: &OutboundRequest) -> Result<Value, Box<HelperError>> {
    Ok(serde_json::json!({
        "url": request.url.as_str(),
        "authorization": fixtures::header(request, "authorization"),
        "accept": fixtures::header(request, "accept"),
        "parts": fixtures::multipart_view(request)?,
    }))
}

fn transcription_outcome(
    result: Result<OperationResult, OpenAiMediaError>,
    request: Value,
) -> HelperResult {
    let (reply, raised) = match result {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::Transcription(reply))) => (
            serde_json::json!({
                "text": reply.response.text.clone(),
                "fields": Value::Object(reply.response.extra.clone()),
                "model": reply.model.clone(),
                "provider": reply.custom_llm_provider,
                "duration_recorded": matches!(reply.duration, StoredDuration::Known(_)),
            }),
            Value::Null,
        ),
        Ok(_) => return Err(failure("Variant", OTHER_OUTCOME)),
        Err(OpenAiMediaError::Transcription(TranscriptionError::Status(error))) => (
            Value::Null,
            serde_json::json!({
                "status": error.status,
                "type": error.error_type.clone(),
                "param": error.param.clone(),
                "code": error.code.clone(),
            }),
        ),
        Err(error) => return Err(leaf_failure(&error)),
    };
    Ok(serde_json::json!({"request": request, "reply": reply, "error": raised}))
}

fn speech_request(
    request: &OutboundRequest,
    options: &SpeechOptions,
) -> Result<Value, Box<HelperError>> {
    let mut extra = serde_json::Map::new();
    for name in options.extra_headers.keys() {
        if let Some(value) = fixtures::header(request, name) {
            extra.insert(name.clone(), Value::String(value));
        }
    }
    Ok(serde_json::json!({
        "method": request.method.as_str(),
        "url": request.url.as_str(),
        "accept": fixtures::header(request, "accept"),
        "authorization": fixtures::header(request, "authorization"),
        "extra_headers": Value::Object(extra),
        "body": fixtures::json_body(request)?,
    }))
}
