use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_video_id_with_provider,
};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::error::VideoError;
use betterlite_openai::videos::reply::VideoObject;
use serde_json::json;

use super::fixture::{Fixture, Recorder, media_error, media_result, video_options};
use crate::openai_media_dispatch::call::{
    OpenAiMediaOperation, VideoCall, VideoContentCall, VideoDeleteCall, VideoListCall,
    VideoOperation, VideoRemixCall, VideoStatusCall,
};
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::openai_media_dispatch::{
    MediaMethod, MediaPayload, OpenAiMediaPrepared, OpenAiMediaRequest, OpenAiMediaResult,
    VideoOutcome, VideoReplyKind,
};
use crate::operation::{OperationRequest, OperationResult};

const API_BASE: &str = "https://video.test/v1";
const LIST_REPLY: &[u8] = br#"{"data":[{"id":"video_a","object":"video","status":"completed","model":"sora-2"}],"object":"list","first_id":"video_a","last_id":"video_a","has_more":false}"#;
const STATUS_REPLY: &[u8] = br#"{"id":"vid-2","object":"video","status":"completed"}"#;
const DELETE_REPLY: &[u8] = br#"{"id":"vid-3","object":"video","status":"deleted"}"#;
const REMIX_REPLY: &[u8] = br#"{"id":"vid-6","object":"video","status":"queued","seconds":"8"}"#;
const BINARY: &[u8] = b"\x00\x00\x00\x18ftypmp42";

fn operation(call: VideoCall) -> VideoOperation {
    VideoOperation {
        api_base: Some(String::from(API_BASE)),
        provider: Some(String::from("openai")),
        call,
    }
}

fn encoded(video_id: &str) -> String {
    encode_video_id_with_provider(video_id, "openai", Some("sora-2"))
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

async fn video_outcome(call: VideoCall) -> VideoOutcome {
    let fixture = Fixture::new(200, b"");
    let result = media_result(run(operation(call), &fixture).await);
    let OpenAiMediaResult::Video(outcome) = result else {
        panic!("a video call stays a video outcome");
    };
    *outcome
}

fn single_video(outcome: &VideoOutcome) -> &VideoObject {
    let VideoReplyKind::Video(video) = &outcome.reply else {
        panic!("a single video reply");
    };
    video
}

pub(super) async fn check() {
    list().await;
    status().await;
    delete().await;
    content().await;
    remix().await;
    path_failure().await;
}

async fn list() {
    let call = VideoCall::List(Box::new(VideoListCall {
        after: Some(encoded("vid-1")),
        limit: Some(2),
        order: Some(String::from("desc")),
        extra_query: video_options(r#"{"user":"u1"}"#),
        reply: LIST_REPLY.to_vec(),
    }));
    let outcome = video_outcome(call).await;
    assert_eq!(outcome.request.method, MediaMethod::Get);
    assert_eq!(outcome.request.url, "https://video.test/v1/videos");
    assert_eq!(outcome.request.body, MediaPayload::Empty);
    assert!(!outcome.request.multipart);
    assert_eq!(
        outcome.request.query,
        video_options(r#"{"after":"vid-1","limit":"2","order":"desc","user":"u1"}"#)
    );
    let VideoReplyKind::List(list) = &outcome.reply else {
        panic!("the list decodes a list");
    };
    assert_eq!(list.data.len(), 1);
    let item_id = list.data[0].id.as_deref().expect("an id");
    let item = decode_video_id_with_provider(item_id);
    assert_eq!(item.video_id, "video_a");
    assert_eq!(item.model_id.as_deref(), Some("sora-2"));
    let first = list
        .first_id
        .as_ref()
        .and_then(|cursor| cursor.as_deref())
        .expect("a first cursor");
    assert_eq!(decode_video_id_with_provider(first).video_id, "video_a");
    assert_eq!(list.has_more, Some(false));
}

async fn status() {
    let call = VideoCall::Status(Box::new(VideoStatusCall {
        video_id: encoded("vid-2"),
        reply: STATUS_REPLY.to_vec(),
    }));
    let outcome = video_outcome(call).await;
    assert_eq!(outcome.request.method, MediaMethod::Get);
    assert_eq!(outcome.request.url, "https://video.test/v1/videos/vid-2");
    let video = single_video(&outcome);
    let decoded = decode_video_id_with_provider(&video.id);
    assert_eq!(decoded.video_id, "vid-2");
    assert_eq!(decoded.model_id.as_deref(), Some(""));
}

async fn delete() {
    let call = VideoCall::Delete(Box::new(VideoDeleteCall {
        video_id: encoded("vid-3"),
        reply: DELETE_REPLY.to_vec(),
    }));
    let outcome = video_outcome(call).await;
    assert_eq!(
        outcome.request,
        OpenAiMediaPrepared {
            method: MediaMethod::Delete,
            url: String::from("https://video.test/v1/videos/vid-3"),
            query: VideoOptions::new(),
            body: MediaPayload::Empty,
            parts: Vec::new(),
            multipart: false,
        }
    );
    let video = single_video(&outcome);
    assert_eq!(video.id, "vid-3");
    assert_eq!(video.status, "deleted");
}

async fn content() {
    let call = VideoCall::Content(Box::new(VideoContentCall {
        video_id: encoded("vid-4"),
        variant: Some(String::from("thumb 1")),
        reply: BINARY.to_vec(),
    }));
    let outcome = video_outcome(call).await;
    assert_eq!(outcome.request.method, MediaMethod::Get);
    assert_eq!(
        outcome.request.url,
        "https://video.test/v1/videos/vid-4/content?variant=thumb%201"
    );
    let VideoReplyKind::Bytes(bytes) = &outcome.reply else {
        panic!("the content stays binary");
    };
    assert_eq!(bytes, &BINARY.to_vec());
}

async fn remix() {
    let call = VideoCall::Remix(Box::new(VideoRemixCall {
        video_id: encoded("vid-5"),
        prompt: String::from("make it rain"),
        extra_body: video_options(r#"{"seconds":"8"}"#),
        reply: REMIX_REPLY.to_vec(),
    }));
    let outcome = video_outcome(call).await;
    assert_eq!(outcome.request.method, MediaMethod::Post);
    assert_eq!(
        outcome.request.url,
        "https://video.test/v1/videos/vid-5/remix"
    );
    let MediaPayload::Options(body) = &outcome.request.body else {
        panic!("the remix sends its body");
    };
    assert_eq!(body.get("prompt"), Some(&json!("make it rain")));
    assert_eq!(body.get("seconds"), Some(&json!("8")));
    let video = single_video(&outcome);
    let decoded = decode_video_id_with_provider(&video.id);
    assert_eq!(decoded.video_id, "vid-6");
    assert_eq!(decoded.model_id.as_deref(), Some(""));
}

async fn path_failure() {
    let fixture = Fixture::new(200, b"");
    let call = VideoCall::Delete(Box::new(VideoDeleteCall {
        video_id: String::new(),
        reply: Vec::new(),
    }));
    let error = media_error(run(operation(call), &fixture).await);
    let OpenAiMediaError::Video(video) = error else {
        panic!("the leaf failure is never turned into an unsupported operation");
    };
    assert!(matches!(video, VideoError::Path { .. }));
    assert_eq!(video.to_string(), "video_id is required");
}
