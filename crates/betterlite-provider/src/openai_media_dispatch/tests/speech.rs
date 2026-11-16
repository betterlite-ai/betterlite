use std::time::Duration;

use betterlite_openai::speech::client::SpeechConnection;
use betterlite_openai::speech::error::SpeechError;
use betterlite_openai::speech::request::{SpeechFormat, SpeechOptions};
use http::Method;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use secrecy::SecretString;

use super::fixture::{Fixture, Recorder, json_body, media_error, media_result, options};
use crate::openai_media_dispatch::call::{OpenAiMediaOperation, SpeechOperation};
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::openai_media_dispatch::{OpenAiMediaRequest, OpenAiMediaResult};
use crate::operation::{OperationRequest, OperationResult};

const SPEECH_BYTES: &[u8] = b"\x00\x01SPEECH";

fn operation(api_key: Option<&str>) -> SpeechOperation {
    SpeechOperation {
        model: String::from("tts-1"),
        input: String::from("hello there"),
        voice: String::from("alloy"),
        options: SpeechOptions {
            response_format: Some(SpeechFormat::Wav),
            speed: Some(Decimal::new(15, 1)),
            instructions: Some(String::from("calm")),
            extra_headers: IndexMap::from([(String::from("x-clip"), String::from("keep"))]),
        },
        connection: SpeechConnection {
            api_key: api_key.map(SecretString::from),
            api_base: Some(String::from("https://speech.test/v1")),
            timeout: Duration::from_secs(30),
            max_retries: 4,
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

async fn run(
    operation: SpeechOperation,
    fixture: &Fixture,
) -> Result<OperationResult, OpenAiMediaError> {
    let mut logger = Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::Speech(Box::new(operation)),
        fixture,
        &mut logger,
    )
    .await
}

pub(super) async fn check() {
    binary_reply().await;
    upstream_failure().await;
}

async fn binary_reply() {
    let fixture = Fixture::new(200, SPEECH_BYTES);
    let result = media_result(run(operation(Some("sk-speech")), &fixture).await);
    let OpenAiMediaResult::Speech(response) = result else {
        panic!("the speech reply stays binary");
    };
    let sent = fixture.only_request();
    assert_eq!(sent.method, Method::POST);
    assert_eq!(sent.url.as_str(), "https://speech.test/v1/audio/speech");
    assert_eq!(
        sent.headers
            .get("accept")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    assert_eq!(
        sent.headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sk-speech")
    );
    assert_eq!(
        sent.headers
            .get("x-clip")
            .and_then(|value| value.to_str().ok()),
        Some("keep")
    );
    assert_eq!(
        json_body(&sent),
        options(
            r#"{"input":"hello there","model":"tts-1","voice":"alloy","response_format":"wav","speed":1.5,"instructions":"calm"}"#
        )
    );
    let status = response.status;
    let bytes = response
        .content(1024)
        .await
        .expect("the speech bytes are read");
    assert_eq!(status, 200);
    assert_eq!(bytes, SPEECH_BYTES.to_vec());
}

async fn upstream_failure() {
    let fixture = Fixture::new(
        429,
        br#"{"error":{"message":"slow down","type":"rate_limit_error","param":null,"code":"rate_limit"}}"#,
    );
    let error = media_error(run(operation(Some("sk-speech")), &fixture).await);
    let OpenAiMediaError::Speech(SpeechError::Status(status)) = error else {
        panic!("the leaf failure is never turned into an unsupported operation");
    };
    assert_eq!(status.status, 429);
    assert_eq!(status.error_type.as_deref(), Some("rate_limit_error"));
    assert_eq!(status.code.as_deref(), Some("rate_limit"));
    assert!(status.message.contains("slow down"));
}
