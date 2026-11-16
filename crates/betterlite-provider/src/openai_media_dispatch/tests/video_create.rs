use std::collections::BTreeMap;

use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_character_id,
};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::create::VideoCreate;
use betterlite_openai::videos::error::VideoError;
use serde_json::json;

use super::fixture::{Fixture, Recorder, media_error, media_result, video_options};
use crate::openai_media_dispatch::call::{
    OpenAiMediaOperation, VideoCall, VideoCreateCall, VideoOperation,
};
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::openai_media_dispatch::{
    MediaMethod, MediaPartBody, MediaPayload, OpenAiMediaRequest, OpenAiMediaResult, VideoOutcome,
    VideoReplyKind,
};
use crate::operation::{OperationRequest, OperationResult};

const API_BASE: &str = "https://video.test/v1";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nreference";
const REPLY: &[u8] = br#"{"id":"video_abc","object":"video","status":"queued","seconds":"4"}"#;

fn characters() -> Vec<BTreeMap<String, String>> {
    let encoded = encode_character_id("char-1", "openai", Some("sora-2"));
    vec![BTreeMap::from([
        (String::from("id"), encoded),
        (String::from("role"), String::from("hero")),
    ])]
}

fn options() -> VideoCreate {
    VideoCreate {
        seconds: Some(String::from("4")),
        size: Some(String::from("720x1280")),
        user: None,
        characters: Some(characters()),
        extra: video_options(r#"{"vendor_flag":true}"#),
    }
}

fn operation(input_reference: Option<Vec<u8>>, reply: &[u8]) -> VideoOperation {
    VideoOperation {
        api_base: Some(String::from(API_BASE)),
        provider: Some(String::from("openai")),
        call: VideoCall::Create(Box::new(VideoCreateCall {
            model: String::from("sora-2"),
            prompt: String::from("a cat surfing"),
            options: options(),
            input_reference,
            reply: reply.to_vec(),
        })),
    }
}

async fn run(
    operation: VideoOperation,
    fixture: &Fixture,
) -> Result<OperationResult, OpenAiMediaError> {
    let mut logger = Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::Video(Box::new(operation)),
        fixture,
        &mut logger,
    )
    .await
}

pub(super) async fn check() {
    multipart().await;
    plain_json().await;
    reply_failure().await;
}

fn body_of(outcome: &VideoOutcome) -> &VideoOptions {
    let MediaPayload::Options(fields) = &outcome.request.body else {
        panic!("the create sends its fields");
    };
    fields
}

async fn multipart() {
    let fixture = Fixture::new(200, REPLY);
    let result = media_result(run(operation(Some(PNG.to_vec()), REPLY), &fixture).await);
    let OpenAiMediaResult::Video(outcome) = result else {
        panic!("the create stays a video outcome");
    };
    assert_eq!(outcome.request.method, MediaMethod::Post);
    assert_eq!(outcome.request.url, "https://video.test/v1/videos");
    assert!(outcome.request.multipart);
    let body = body_of(&outcome);
    assert_eq!(body.get("model"), Some(&json!("sora-2")));
    assert_eq!(body.get("prompt"), Some(&json!("a cat surfing")));
    assert_eq!(body.get("seconds"), Some(&json!("4")));
    assert_eq!(body.get("size"), Some(&json!("720x1280")));
    assert_eq!(body.get("vendor_flag"), Some(&json!(true)));
    let characters = body
        .get("characters")
        .and_then(|value| value.as_array())
        .expect("the characters ride along");
    assert_eq!(characters[0].get("id"), Some(&json!("char-1")));
    assert_eq!(characters[0].get("role"), Some(&json!("hero")));
    assert_eq!(outcome.request.parts.len(), 1);
    let part = &outcome.request.parts[0];
    assert_eq!(part.field, "input_reference");
    assert_eq!(part.filename.as_deref(), Some("input_reference.png"));
    assert_eq!(part.content_type.as_deref(), Some("image/png"));
    assert_eq!(part.body, MediaPartBody::Bytes(PNG.to_vec()));
    let VideoReplyKind::Video(video) = &outcome.reply else {
        panic!("the create decodes one video object");
    };
    let decoded = decode_video_id_with_provider(&video.id);
    assert_eq!(decoded.video_id, "video_abc");
    assert_eq!(decoded.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(decoded.model_id.as_deref(), Some("sora-2"));
    let usage = video.usage.as_ref().expect("the create adds usage");
    assert_eq!(usage.get("duration_seconds"), Some(&json!(4.0)));
}

async fn plain_json() {
    let fixture = Fixture::new(200, REPLY);
    let result = media_result(run(operation(None, REPLY), &fixture).await);
    let OpenAiMediaResult::Video(outcome) = result else {
        panic!("the create stays a video outcome");
    };
    assert!(outcome.request.multipart);
    assert!(outcome.request.parts.is_empty());
    let body = body_of(&outcome);
    assert!(!body.contains_key("input_reference"));
}

async fn reply_failure() {
    let fixture = Fixture::new(200, b"not a video");
    let error = media_error(run(operation(None, b"not a video"), &fixture).await);
    let OpenAiMediaError::Video(video) = error else {
        panic!("the leaf failure is never turned into an unsupported operation");
    };
    assert!(matches!(video, VideoError::Reply { .. }));
}
