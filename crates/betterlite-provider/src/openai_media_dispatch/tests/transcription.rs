use std::time::Duration;

use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::transcription::client::TranscriptionConnection;
use betterlite_openai::transcription::config::TranscriptionFamily;
use betterlite_openai::transcription::error::TranscriptionError;
use betterlite_openai::transcription::reply::StoredDuration;
use http::Method;
use secrecy::SecretString;
use uuid::Uuid;

use super::fixture::{Fixture, Recorder, boundary, media_error, media_result, options, parts};
use crate::openai_media_dispatch::call::{OpenAiMediaOperation, TranscriptionOperation};
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::openai_media_dispatch::{OpenAiMediaRequest, OpenAiMediaResult};
use crate::operation::{OperationRequest, OperationResult};

const WAV: &[u8] = b"RIFF\x24\x00\x00\x00WAVEfmt ";
const REPLY: &[u8] = br#"{"text":"hello world","duration":1.5,"language":"en"}"#;

fn operation(api_key: Option<&str>) -> TranscriptionOperation {
    TranscriptionOperation {
        input: String::from("clip.wav"),
        file: ProcessedAudioFile {
            file_content: WAV.to_vec(),
            filename: String::from("clip.wav"),
            content_type: String::from("audio/wav"),
        },
        connection: TranscriptionConnection {
            api_key: api_key.map(SecretString::from),
            environment_api_key: None,
            api_base: Some(String::from("https://audio.test/v1")),
            timeout: Duration::from_secs(30),
            max_retries: 2,
            shared_session: None,
        },
        boundary: Uuid::from_u128(7),
        model: String::from("whisper-1"),
        family: Some(TranscriptionFamily::Whisper),
        optional_params: options(r#"{"language":"en"}"#),
        atranscription: false,
    }
}

async fn run(
    operation: TranscriptionOperation,
    fixture: &Fixture,
) -> Result<OperationResult, OpenAiMediaError> {
    let mut logger = Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::Transcription(Box::new(operation)),
        fixture,
        &mut logger,
    )
    .await
}

pub(super) async fn check() {
    projection().await;
    missing_key().await;
}

async fn projection() {
    let fixture = Fixture::new(200, REPLY);
    let result = media_result(run(operation(Some("sk-transcription")), &fixture).await);
    let OpenAiMediaResult::Transcription(reply) = result else {
        panic!("the transcription stays its own reply");
    };
    let sent = fixture.only_request();
    assert_eq!(sent.method, Method::POST);
    assert_eq!(
        sent.url.as_str(),
        "https://audio.test/v1/audio/transcriptions"
    );
    assert_eq!(boundary(&sent), "00000000-0000-0000-0000-000000000007");
    let form = parts(&sent);
    let names: Vec<&str> = form.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(names, ["model", "language", "response_format", "file"]);
    assert_eq!(form[0].data, b"whisper-1".to_vec());
    assert_eq!(form[1].data, b"en".to_vec());
    assert_eq!(form[2].data, b"verbose_json".to_vec());
    assert_eq!(form[3].file_name.as_deref(), Some("clip.wav"));
    assert_eq!(form[3].content_type.as_deref(), Some("audio/wav"));
    assert_eq!(form[3].data, WAV.to_vec());
    assert_eq!(reply.response.text.as_deref(), Some("hello world"));
    assert_eq!(reply.custom_llm_provider, "openai");
    assert_eq!(reply.model.as_deref(), Some("whisper-1"));
    assert_eq!(reply.duration, StoredDuration::Absent);
}

async fn missing_key() {
    let fixture = Fixture::new(200, REPLY);
    let error = media_error(run(operation(None), &fixture).await);
    let OpenAiMediaError::Transcription(transcription) = error else {
        panic!("the leaf failure is never turned into an unsupported operation");
    };
    assert!(matches!(transcription, TranscriptionError::MissingApiKey));
}
