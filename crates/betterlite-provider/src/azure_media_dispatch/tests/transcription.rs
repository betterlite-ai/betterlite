use std::io;
use std::path::Path;

use betterlite_azure::transcription::native::NativeError;
use betterlite_azure::transcription::native::reply::NativeTranscriptionReply;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::AuthorizedAudioFiles;
use betterlite_openai::api::transcription_contract::AudioTranscriptionData;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::transcription::reply::{StoredDuration, TranscriptionReply};

use super::*;

pub(super) async fn check() {
    openai_sync().await;
    openai_async().await;
    openai_failure().await;
    native_success().await;
    native_content_type().await;
    native_recognition_failure().await;
    native_http_failure().await;
    native_missing_key().await;
    native_bytes_guard();
}

fn transcription_result(outcome: &Outcome) -> &TranscriptionReply {
    let Ok(OperationResult::AzureMedia(AzureMediaResult::Transcription(reply))) = &outcome.result
    else {
        panic!("the transcription arm returns its reply");
    };
    reply
}

fn native_result(outcome: &Outcome) -> &NativeTranscriptionReply {
    let Ok(OperationResult::AzureMedia(AzureMediaResult::NativeTranscription(reply))) =
        &outcome.result
    else {
        panic!("the native transcription arm returns its reply");
    };
    reply
}

async fn openai_sync() {
    let outcome = exchange(
        AzureMediaOperation::Transcription(Box::new(transcription_operation(false))),
        vec![
            Reply::json(200, r#"{"text":"hello"}"#).with_header("content-type", "application/json"),
        ],
    )
    .await;
    let reply = transcription_result(&outcome);
    assert_eq!(reply.response.text.as_deref(), Some("hello"));
    assert_eq!(reply.model.as_deref(), Some("whisper"));
    assert_eq!(reply.custom_llm_provider, "azure");
    assert_eq!(reply.duration, StoredDuration::Absent);
    assert_eq!(outcome.logger.audio_pre, 1);
    assert_eq!(outcome.logger.audio_post, 1);
    let requests = outcome.fixture.outbound();
    let sent = &requests[0];
    assert_eq!(sent.method, Method::POST);
    assert_eq!(
        sent.url.as_str(),
        format!("{AZURE}/openai/deployments/whisper/audio/transcriptions?api-version=2024-06-01")
    );
    let RequestBody::Multipart { parts, boundary } = &sent.body else {
        panic!("the transcription request is multipart");
    };
    assert_eq!(boundary, "00000000-0000-0000-0000-00000000000b");
    let model = parts
        .iter()
        .find(|part| part.name == "model")
        .expect("the request carries the model");
    assert_eq!(model.data, b"whisper");
    let file = parts
        .iter()
        .find(|part| part.name == "file")
        .expect("the request carries the audio");
    assert_eq!(file.data, b"abc");
    assert_eq!(file.file_name.as_deref(), Some("a.wav"));
    assert_eq!(header(&sent.headers, "api-key"), Some("sk-azure-fixture"));
}

async fn openai_async() {
    let outcome = exchange(
        AzureMediaOperation::Transcription(Box::new(transcription_operation(true))),
        vec![
            Reply::json(200, r#"{"text":"hello"}"#).with_header("content-type", "application/json"),
        ],
    )
    .await;
    let reply = transcription_result(&outcome);
    assert_eq!(reply.response.text.as_deref(), Some("hello"));
    assert_eq!(reply.duration, StoredDuration::Absent);
    assert_eq!(outcome.logger.audio_pre, 1);
    assert_eq!(outcome.logger.audio_post, 1);
    assert_eq!(outcome.fixture.outbound().len(), 1);
}

async fn openai_failure() {
    let body =
        r#"{"error":{"message":"slow down","type":"rate_limit_error","param":null,"code":"429"}}"#;
    let outcome = exchange(
        AzureMediaOperation::Transcription(Box::new(transcription_operation(false))),
        vec![Reply::json(429, body).with_header("content-type", "application/json")],
    )
    .await;
    let error = error_of(&outcome);
    assert_eq!(error.status(), Some(429));
    assert!(matches!(error, AzureMediaError::Transcription(_)));
    assert!(error.to_string().contains("slow down"));
}

async fn native_success() {
    let outcome = exchange(
        AzureMediaOperation::NativeTranscription(Box::new(native_operation())),
        vec![Reply::json(
            200,
            r#"{"RecognitionStatus":"Success","DisplayText":"hi"}"#,
        )],
    )
    .await;
    let reply = native_result(&outcome);
    assert_eq!(reply.response.text.as_deref(), Some("hi"));
    let Json::Object(hidden) = &reply.hidden_params else {
        panic!("the native reply keeps its whole JSON document");
    };
    assert_eq!(
        hidden.get("DisplayText"),
        Some(&Json::Text(String::from("hi")))
    );
    let requests = outcome.fixture.outbound();
    let sent = &requests[0];
    assert_eq!(sent.method, Method::POST);
    assert_eq!(
        sent.url.as_str(),
        "https://westus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=en-US&format=simple"
    );
    assert_eq!(
        header(&sent.headers, "Ocp-Apim-Subscription-Key"),
        Some("speech-key")
    );
    assert_eq!(header(&sent.headers, "content-type"), Some("audio/wav"));
    assert_eq!(header(&sent.headers, "accept"), Some("application/json"));
    let RequestBody::Bytes { content_type, data } = &sent.body else {
        panic!("the native request body is the raw audio");
    };
    assert_eq!(
        content_type.to_str().expect("the content type is text"),
        "audio/wav"
    );
    assert_eq!(data, b"abc");
}

// the provider config's transformed content type replaces the validated default
async fn native_content_type() {
    let operation = NativeTranscriptionOperation {
        filename: String::from("a.mp3"),
        ..native_operation()
    };
    let outcome = exchange(
        AzureMediaOperation::NativeTranscription(Box::new(operation)),
        vec![Reply::json(
            200,
            r#"{"RecognitionStatus":"Success","DisplayText":"hi"}"#,
        )],
    )
    .await;
    let sent = &outcome.fixture.outbound()[0];
    assert_eq!(header(&sent.headers, "content-type"), Some("audio/mpeg"));
    let RequestBody::Bytes { content_type, .. } = &sent.body else {
        panic!("the native request body is the raw audio");
    };
    assert_eq!(
        content_type.to_str().expect("the content type is text"),
        "audio/mpeg"
    );
}

async fn native_recognition_failure() {
    let outcome = exchange(
        AzureMediaOperation::NativeTranscription(Box::new(native_operation())),
        vec![Reply::json(200, r#"{"RecognitionStatus":"Error"}"#)],
    )
    .await;
    let error = error_of(&outcome);
    assert_eq!(error.status(), Some(200));
    assert!(matches!(
        error,
        AzureMediaError::Native(NativeError::RecognitionStatus { .. })
    ));
    assert!(error.to_string().contains("RecognitionStatus=Error"));
}

async fn native_http_failure() {
    let outcome = exchange(
        AzureMediaOperation::NativeTranscription(Box::new(native_operation())),
        vec![Reply::json(401, "denied").with_header("x-request-id", "req-1")],
    )
    .await;
    let error = error_of(&outcome);
    assert_eq!(error.status(), Some(401));
    let AzureMediaError::Native(native) = &error else {
        panic!("a failed native reply raises the native error");
    };
    assert_eq!(native.to_string(), "denied");
    let headers = native
        .headers()
        .expect("the native failure carries the reply headers");
    assert_eq!(
        headers.get("x-request-id").map(String::as_str),
        Some("req-1")
    );
}

async fn native_missing_key() {
    let operation = NativeTranscriptionOperation {
        environment_key: None,
        api_key: None,
        ..native_operation()
    };
    let outcome = exchange(
        AzureMediaOperation::NativeTranscription(Box::new(operation)),
        Vec::new(),
    )
    .await;
    let error = error_of(&outcome);
    assert_eq!(error.status(), Some(401));
    assert!(matches!(
        error,
        AzureMediaError::Native(NativeError::MissingApiKey)
    ));
    assert!(outcome.fixture.outbound().is_empty());
}

// the native transform always returns the audio bytes, so the other data variant is a hard error
fn native_bytes_guard() {
    let error =
        super::super::transcription::native_bytes(AudioTranscriptionData::Object(Object::new()))
            .expect_err("an object cannot be the native request body");
    assert!(matches!(error, AzureMediaError::NativeRequest));
    assert_eq!(error.status(), None);
}

// the native transform reads no path, and a path reaching it is a caller error rather than a filesystem read
#[test]
fn native_files_reject_any_path() {
    let path = Path::new("a.wav");
    let read = super::super::transcription::NoFiles
        .read(path)
        .expect_err("the native transform reads no path");
    assert_eq!(read.kind(), io::ErrorKind::NotFound);
    assert_eq!(read.to_string(), "a.wav");
    let open = super::super::transcription::NoFiles
        .open(path)
        .err()
        .expect("the native transform opens no path");
    assert_eq!(open.kind(), io::ErrorKind::NotFound);
}
