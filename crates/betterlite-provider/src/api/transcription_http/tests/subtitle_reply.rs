use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::{
    AudioTranscriptionData, TranscriptionResponse,
};
use betterlite_openai::openai_base_config::Object;
use serde_json::Number;

use super::support::{
    Entry, Events, Fixture, RecordingConfig, Reply, Source, Wire, call, deps, extra_words, json,
    object, record,
};
use crate::api::transcription_http::error::TranscriptionHttpError;
use crate::api::transcription_http::reply::TranscriptionReply;

const WORDS: &str = r#"{"word":"Hello","start":0,"end":1}"#;

#[tokio::test]
async fn transcription_subtitle_reply() {
    assert_srt_document().await;
    assert_vtt_document().await;
    assert_invalid_timings_keep_text().await;
    assert_missing_timings_keep_text().await;
    assert_missing_words_keep_text().await;
    assert_disabled_synthesis_keeps_words().await;
    assert_other_formats_keep_words().await;
}

async fn run_reply(
    response: &str,
    format: Json,
    subtitles: bool,
) -> Result<TranscriptionReply, TranscriptionHttpError> {
    let events = Events::new();
    let reply = Reply::json(200, response)
        .with_header("content-type", "application/json")
        .with_header("x-request-id", "transcribe_1");
    let wire = Wire::replying(reply, &events);
    let source = Source::new(wire);
    let mut config = RecordingConfig::new(
        record(AudioTranscriptionData::Object(Object::new())),
        &events,
    );
    config.subtitles = subtitles;
    config.response =
        serde_json::from_str(response).expect("the response fixture is a transcription response");
    let mut optional = Object::new();
    optional.insert(String::from("response_format"), format);
    let litellm = object("{}");
    let request_call = call(&optional, &litellm);
    let mut fixture = Fixture::new("transcription");
    fixture
        .run(
            &events,
            &deps(&config, &source),
            &request_call,
            Entry::Audio,
        )
        .await
}

fn words_response() -> String {
    format!(r#"{{"text":"old","words":[{WORDS}]}}"#)
}

async fn assert_srt_document() {
    let reply = run_reply(&words_response(), Json::Text(String::from("srt")), true)
        .await
        .expect("the subtitle call succeeds");
    assert_eq!(
        reply.response.text.as_deref(),
        Some("1\n00:00:00,000 --> 00:00:01,000\nHello\n")
    );
    assert_eq!(extra_words(&reply.response), None);
    assert_eq!(
        reply.provider_headers.headers.get("x-request-id"),
        Some(&String::from("transcribe_1"))
    );
    assert_eq!(
        reply
            .provider_headers
            .additional_headers
            .get("llm_provider-x-request-id"),
        Some(&Json::Text(String::from("transcribe_1")))
    );
}

async fn assert_vtt_document() {
    let reply = run_reply(&words_response(), Json::Text(String::from("vtt")), true)
        .await
        .expect("the subtitle call succeeds");
    assert_eq!(
        reply.response.text.as_deref(),
        Some("WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nHello\n")
    );
    assert_eq!(extra_words(&reply.response), None);
}

async fn assert_invalid_timings_keep_text() {
    let response = r#"{"text":"old","words":[{"word":"Hello","start":"nope","end":1}]}"#;
    let reply = run_reply(response, Json::Text(String::from("srt")), true)
        .await
        .expect("an unusable timing is not an error");
    assert_eq!(reply.response.text.as_deref(), Some("old"));
    assert_eq!(extra_words(&reply.response), None);
}

async fn assert_missing_timings_keep_text() {
    let response = r#"{"text":"old","words":[{"word":"Hello"}]}"#;
    let reply = run_reply(response, Json::Text(String::from("srt")), true)
        .await
        .expect("a missing timing is not an error");
    assert_eq!(reply.response.text.as_deref(), Some("old"));
    assert_eq!(extra_words(&reply.response), None);
}

async fn assert_missing_words_keep_text() {
    let reply = run_reply(r#"{"text":"old"}"#, Json::Text(String::from("srt")), true)
        .await
        .expect("a response without words is not an error");
    assert_eq!(reply.response.text.as_deref(), Some("old"));
    assert_eq!(extra_words(&reply.response), None);
}

async fn assert_disabled_synthesis_keeps_words() {
    let reply = run_reply(&words_response(), Json::Text(String::from("srt")), false)
        .await
        .expect("the call succeeds");
    assert_eq!(reply.response.text.as_deref(), Some("old"));
    assert_eq!(
        extra_words(&reply.response),
        Some(json(&format!("[{WORDS}]")))
    );
}

async fn assert_other_formats_keep_words() {
    let expected = Some(json(&format!("[{WORDS}]")));
    let json_format = run_reply(&words_response(), Json::Text(String::from("json")), true)
        .await
        .expect("the call succeeds");
    assert_eq!(json_format.response.text.as_deref(), Some("old"));
    assert_eq!(extra_words(&json_format.response), expected);
    let upper = run_reply(&words_response(), Json::Text(String::from("SRT")), true)
        .await
        .expect("the call succeeds");
    assert_eq!(upper.response.text.as_deref(), Some("old"));
    assert_eq!(extra_words(&upper.response), expected);
    let number = run_reply(&words_response(), Json::Number(Number::from(1)), true)
        .await
        .expect("the call succeeds");
    assert_eq!(number.response.text.as_deref(), Some("old"));
    assert_eq!(extra_words(&number.response), expected);
}
