use std::time::Duration;

use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::speech::client::SpeechConnection;
use betterlite_openai::speech::error::SpeechError;
use betterlite_openai::transcription::client::TranscriptionConnection;
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_openai::transcription::error::TranscriptionError;
use betterlite_openai::transcription::reply::StoredDuration;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_media_dispatch::OpenAiMediaResult;
use betterlite_provider::openai_media_dispatch::call::{
    OpenAiMediaOperation, SpeechOperation, TranscriptionOperation,
};
use betterlite_provider::openai_media_dispatch::error::OpenAiMediaError;
use betterlite_provider::operation::OperationResult;
use http::HeaderMap;
use http::HeaderValue;
use http::header::CONTENT_TYPE;
use secrecy::SecretString;
use serde_json::Value;
use uuid::Uuid;

use super::fixtures::{self, Fixture};
use super::{
    OTHER_OUTCOME, SpeechCase, TranscriptionCase, TranscriptionFamilyCase, failure, leaf_failure,
    run,
};

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
    let result = run(&fixture, OpenAiMediaOperation::Speech(Box::new(operation))).await;
    let sent = fixtures::only_request(&fixture)?;
    let mut extra = serde_json::Map::new();
    for name in case.options.extra_headers.keys() {
        if let Some(value) = fixtures::header(&sent, name) {
            extra.insert(name.clone(), Value::String(value));
        }
    }
    let request = serde_json::json!({
        "method": sent.method.as_str(),
        "url": sent.url.as_str(),
        "accept": fixtures::header(&sent, "accept"),
        "authorization": fixtures::header(&sent, "authorization"),
        "extra_headers": Value::Object(extra),
        "body": fixtures::json_body(&sent)?,
    });
    audio_outcome(result, request).await
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

// Both audio leaves answer with the request they sent and the reply or the typed status error they got.
async fn audio_outcome(
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
                "body": String::from_utf8(body).map_err(|error| failure("Utf8", error))?,
                "error": Value::Null,
            }))
        }
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::Transcription(reply))) => {
            Ok(serde_json::json!({
                "request": request,
                "reply": {
                    "text": reply.response.text,
                    "fields": Value::Object(reply.response.extra),
                    "model": reply.model,
                    "provider": reply.custom_llm_provider,
                    "duration_recorded": matches!(reply.duration, StoredDuration::Known(_)),
                },
                "error": Value::Null,
            }))
        }
        Err(OpenAiMediaError::Speech(SpeechError::Status(raised))) => Ok(serde_json::json!({
            "request": request,
            "status": Value::Null,
            "content_type": Value::Null,
            "body": Value::Null,
            "error": {
                "status": raised.status,
                "type": raised.error_type,
                "param": raised.param,
                "code": raised.code,
            },
        })),
        Err(OpenAiMediaError::Transcription(TranscriptionError::Status(raised))) => {
            Ok(serde_json::json!({
                "request": request,
                "reply": Value::Null,
                "error": {
                    "status": raised.status,
                    "type": raised.error_type,
                    "param": raised.param,
                    "code": raised.code,
                },
            }))
        }
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Err(leaf_failure(&error)),
    }
}

pub(super) async fn transcription(case: &TranscriptionCase) -> HelperResult {
    let operation = transcription_operation(case);
    let fixture = Fixture::new(case.status, case.reply.as_bytes());
    let result = run(
        &fixture,
        OpenAiMediaOperation::Transcription(Box::new(operation)),
    )
    .await;
    let sent = fixtures::only_request(&fixture)?;
    let request = serde_json::json!({
        "url": sent.url.as_str(),
        "authorization": fixtures::header(&sent, "authorization"),
        "accept": fixtures::header(&sent, "accept"),
        "parts": fixtures::multipart_view(&sent)?,
    });
    audio_outcome(result, request).await
}

fn transcription_operation(case: &TranscriptionCase) -> TranscriptionOperation {
    let family = match case.family {
        TranscriptionFamilyCase::Whisper => TranscriptionFamily::Whisper,
        TranscriptionFamilyCase::Gpt => TranscriptionFamily::Gpt,
    };
    TranscriptionOperation {
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
    }
}
