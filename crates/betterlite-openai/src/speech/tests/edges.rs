use super::cases::{audio_ok, call};
use super::fixture::{Fixture, Reply, connection};
use crate::speech::error::SpeechError;
use crate::speech::request::SpeechOptions;

fn failure(chunks: Vec<Vec<u8>>) -> Fixture {
    Fixture::new(Reply {
        status: 500,
        headers: Vec::new(),
        chunks,
    })
}

#[tokio::test]
async fn omitted_api_base_uses_the_openai_endpoint() {
    let mut settings = connection();
    settings.api_base = None;
    let fixture = audio_ok(vec![vec![1]]);
    let (outcome, recorder) =
        call(&fixture, &SpeechOptions::default(), settings, false, "Hi").await;
    outcome.expect("the default endpoint serves the call");
    assert_eq!(
        fixture.request().url.as_str(),
        "https://api.openai.com/v1/audio/speech"
    );
    assert_eq!(recorder.records[0].api_base, "https://api.openai.com/v1/");
}

#[tokio::test]
async fn large_json_error_keeps_type_param_and_code() {
    let padding = "x".repeat(100 * 1024);
    let text = format!(
        r#"{{"error":{{"message":"{padding}","type":"invalid_request_error","param":"voice","code":"voice_not_found"}}}}"#
    );
    let (head, tail) = text.as_bytes().split_at(70 * 1024);
    let fixture = failure(vec![head.to_vec(), tail.to_vec()]);
    let (outcome, _) = call(
        &fixture,
        &SpeechOptions::default(),
        connection(),
        false,
        "Hi",
    )
    .await;
    let Err(SpeechError::Status(status)) = outcome else {
        panic!("the 500 is the SDK's status error");
    };
    assert_eq!(status.error_type.as_deref(), Some("invalid_request_error"));
    assert_eq!(status.param.as_deref(), Some("voice"));
    assert_eq!(status.code.as_deref(), Some("voice_not_found"));
}

#[tokio::test]
async fn error_body_is_read_up_to_one_mebibyte() {
    let chunks = vec![vec![b'a'; 256 * 1024]; 8];
    let fixture = failure(chunks);
    let (outcome, _) = call(
        &fixture,
        &SpeechOptions::default(),
        connection(),
        false,
        "Hi",
    )
    .await;
    let Err(SpeechError::Status(status)) = outcome else {
        panic!("the 500 is the SDK's status error");
    };
    assert_eq!(status.message.len(), 1024 * 1024);
}
