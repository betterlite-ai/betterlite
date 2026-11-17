use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::ids::{
    decode_video_id_with_provider, encode_character_id, encode_video_id_with_provider,
};
use betterlite_openai::video_edits::error::VideoEditError;
use indexmap::IndexMap;

use super::fixture::{Fixture, Recorder, media_error, media_result, options};
use crate::openai_media_dispatch::call::{
    CharacterCreateInput, CharacterGetInput, OpenAiMediaOperation, VideoEditCall, VideoEditInput,
    VideoEditOperation, VideoExtensionInput,
};
use crate::openai_media_dispatch::error::OpenAiMediaError;
use crate::openai_media_dispatch::execute::dispatch;
use crate::openai_media_dispatch::{
    MediaMethod, MediaPartBody, MediaPayload, OpenAiMediaRequest, OpenAiMediaResult,
    VideoEditOutcome, VideoEditReplyKind,
};
use crate::operation::{OperationRequest, OperationResult};

const API_BASE: &str = "https://video.test/v1";
const VIDEO: &[u8] = b"\x00\x00\x00\x18ftypmp42";
const EDIT_REPLY: &[u8] =
    br#"{"id":"video_edit","object":"video","status":"completed","seconds":"4"}"#;
const EXTENSION_REPLY: &[u8] =
    br#"{"id":"video_long","object":"video","status":"queued","seconds":"8"}"#;
const CHARACTER_REPLY: &[u8] =
    br#"{"id":"char_9","object":"character","created_at":1700000000,"name":"hero"}"#;

fn encoded(video_id: &str) -> String {
    encode_video_id_with_provider(video_id, "openai", Some("sora-2"))
}

fn operation(call: VideoEditCall) -> VideoEditOperation {
    VideoEditOperation {
        api_base: Some(String::from(API_BASE)),
        provider: Some(String::from("openai")),
        call,
    }
}

async fn run(
    operation: VideoEditOperation,
    fixture: &Fixture,
) -> Result<OperationResult, OpenAiMediaError> {
    let mut logger = Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        OpenAiMediaOperation::VideoEdit(Box::new(operation)),
        fixture,
        &mut logger,
    )
    .await
}

fn edit(video_file: Option<Vec<u8>>, reply: &[u8]) -> VideoEditCall {
    VideoEditCall::Edit(Box::new(VideoEditInput {
        prompt: String::from("add a hat"),
        video_id: encoded("video_1"),
        video_file,
        extra_body: Some(options(r#"{"seconds":"4"}"#)),
        reply: reply.to_vec(),
    }))
}

fn body_of(outcome: &VideoEditOutcome) -> &IndexMap<String, Json> {
    let MediaPayload::Body(body) = &outcome.request.body else {
        panic!("the extended operations send their body");
    };
    body
}

pub(super) async fn check() {
    edit_by_id().await;
    edit_with_file().await;
    extension().await;
    create_character().await;
    get_character().await;
    failures().await;
}

async fn edit_by_id() {
    let fixture = Fixture::new(200, b"");
    let result = media_result(run(operation(edit(None, EDIT_REPLY)), &fixture).await);
    let OpenAiMediaResult::VideoEdit(outcome) = result else {
        panic!("the edit stays an extended video outcome");
    };
    assert_eq!(outcome.request.method, MediaMethod::Post);
    assert_eq!(outcome.request.url, "https://video.test/v1/videos/edits");
    assert!(!outcome.request.multipart);
    assert!(outcome.request.parts.is_empty());
    let body = body_of(&outcome);
    assert_eq!(
        body.get("prompt"),
        Some(&Json::Text(String::from("add a hat")))
    );
    assert_eq!(
        body.get("video"),
        Some(&Json::Object(IndexMap::from([(
            String::from("id"),
            Json::Text(String::from("video_1"))
        )])))
    );
    assert_eq!(body.get("seconds"), Some(&Json::Text(String::from("4"))));
    let VideoEditReplyKind::Video(video) = &outcome.reply else {
        panic!("the edit decodes one video object");
    };
    let decoded = decode_video_id_with_provider(&video.id);
    assert_eq!(decoded.video_id, "video_edit");
    assert_eq!(decoded.model_id.as_deref(), Some(""));
}

async fn edit_with_file() {
    let fixture = Fixture::new(200, b"");
    let result =
        media_result(run(operation(edit(Some(VIDEO.to_vec()), EDIT_REPLY)), &fixture).await);
    let OpenAiMediaResult::VideoEdit(outcome) = result else {
        panic!("the edit stays an extended video outcome");
    };
    assert!(outcome.request.multipart);
    assert_eq!(outcome.request.parts.len(), 1);
    let part = &outcome.request.parts[0];
    assert_eq!(part.field, "video");
    assert_eq!(part.filename.as_deref(), Some("input_video.mp4"));
    assert_eq!(part.content_type.as_deref(), Some("video/mp4"));
    assert_eq!(part.body, MediaPartBody::Bytes(VIDEO.to_vec()));
    assert!(!body_of(&outcome).contains_key("video"));
}

async fn extension() {
    let fixture = Fixture::new(200, b"");
    let call = VideoEditCall::Extension(Box::new(VideoExtensionInput {
        prompt: String::from("make it longer"),
        video_id: encoded("video_2"),
        seconds: String::from("8"),
        extra_body: None,
        reply: EXTENSION_REPLY.to_vec(),
    }));
    let result = media_result(run(operation(call), &fixture).await);
    let OpenAiMediaResult::VideoEdit(outcome) = result else {
        panic!("the extension stays an extended video outcome");
    };
    assert_eq!(outcome.request.method, MediaMethod::Post);
    assert_eq!(
        outcome.request.url,
        "https://video.test/v1/videos/extensions"
    );
    assert!(!outcome.request.multipart);
    let body = body_of(&outcome);
    assert_eq!(body.get("seconds"), Some(&Json::Text(String::from("8"))));
    assert_eq!(
        body.get("video"),
        Some(&Json::Object(IndexMap::from([(
            String::from("id"),
            Json::Text(String::from("video_2"))
        )])))
    );
    let VideoEditReplyKind::Video(video) = &outcome.reply else {
        panic!("the extension decodes one video object");
    };
    assert_eq!(
        decode_video_id_with_provider(&video.id).video_id,
        "video_long"
    );
}

async fn create_character() {
    let fixture = Fixture::new(200, b"");
    let call = VideoEditCall::CreateCharacter(Box::new(CharacterCreateInput {
        name: String::from("hero"),
        video: VIDEO.to_vec(),
        reply: CHARACTER_REPLY.to_vec(),
    }));
    let result = media_result(run(operation(call), &fixture).await);
    let OpenAiMediaResult::VideoEdit(outcome) = result else {
        panic!("the character stays an extended video outcome");
    };
    assert_eq!(outcome.request.method, MediaMethod::Post);
    assert_eq!(
        outcome.request.url,
        "https://video.test/v1/videos/characters"
    );
    assert!(outcome.request.multipart);
    assert_eq!(outcome.request.parts.len(), 2);
    let name = &outcome.request.parts[0];
    assert_eq!(name.field, "name");
    assert_eq!(name.filename, None);
    assert_eq!(name.content_type, None);
    assert_eq!(name.body, MediaPartBody::Text(String::from("hero")));
    let upload = &outcome.request.parts[1];
    assert_eq!(upload.field, "video");
    assert_eq!(upload.filename.as_deref(), Some("input_video.mp4"));
    assert_eq!(upload.body, MediaPartBody::Bytes(VIDEO.to_vec()));
    let VideoEditReplyKind::Character(character) = &outcome.reply else {
        panic!("the create decodes one character object");
    };
    assert_eq!(character.id, "char_9");
    assert_eq!(character.name, "hero");
}

async fn get_character() {
    let fixture = Fixture::new(200, b"");
    let call = VideoEditCall::GetCharacter(Box::new(CharacterGetInput {
        character_id: encode_character_id("char_7", "openai", None),
        reply: CHARACTER_REPLY.to_vec(),
    }));
    let result = media_result(run(operation(call), &fixture).await);
    let OpenAiMediaResult::VideoEdit(outcome) = result else {
        panic!("the character stays an extended video outcome");
    };
    assert_eq!(outcome.request.method, MediaMethod::Get);
    assert_eq!(
        outcome.request.url,
        "https://video.test/v1/videos/characters/char_7"
    );
    assert!(!outcome.request.multipart);
    let VideoEditReplyKind::Character(character) = &outcome.reply else {
        panic!("the get decodes one character object");
    };
    assert_eq!(character.id, "char_9");
}

async fn failures() {
    missing_character_id().await;
    bad_reply().await;
}

async fn missing_character_id() {
    let fixture = Fixture::new(200, b"");
    let call = VideoEditCall::GetCharacter(Box::new(CharacterGetInput {
        character_id: String::new(),
        reply: Vec::new(),
    }));
    let error = media_error(run(operation(call), &fixture).await);
    let OpenAiMediaError::VideoEdit(video_edit) = error else {
        panic!("the leaf failure is never turned into an unsupported operation");
    };
    assert!(matches!(video_edit, VideoEditError::CharacterId(_)));
}

async fn bad_reply() {
    let fixture = Fixture::new(200, b"");
    let error = media_error(run(operation(edit(None, b"[]")), &fixture).await);
    let OpenAiMediaError::VideoEdit(video_edit) = error else {
        panic!("the reply failure stays a leaf failure");
    };
    assert!(matches!(video_edit, VideoEditError::VideoReply(_)));
}
