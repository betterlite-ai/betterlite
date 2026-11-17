use betterlite_models::decisions::Json;
use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::video_edits::character::{
    CharacterPart,
    CreateCharacter,
    // ast-grep-ignore: no-long-names
    transform_video_create_character_request as character_request,
    // ast-grep-ignore: no-long-names
    transform_video_get_character_request as character_get_request,
};
use betterlite_openai::video_edits::edit::{
    VideoEdit, VideoEditRequest, transform_video_edit_request as edit_request,
};
use betterlite_openai::video_edits::extension::{
    VideoExtension,
    VideoExtensionRequest,
    // ast-grep-ignore: no-long-names
    transform_video_extension_request as extension_request,
};
use betterlite_openai::video_edits::response::{
    CharacterObject, VideoObject, VideoReply, decode_character_reply, transform_video_response,
};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::complete_url;
use indexmap::IndexMap;

use super::call::{
    CharacterCreateInput, CharacterGetInput, VideoEditCall, VideoEditInput, VideoEditOperation,
    VideoExtensionInput,
};
use super::error::OpenAiMediaError;
use super::parts::media_part;
use super::{
    MediaMethod, MediaPart, MediaPartBody, MediaPayload, OpenAiMediaPrepared, VideoEditOutcome,
    VideoEditReplyKind,
};

type VideoEditResult = Result<VideoEditOutcome, OpenAiMediaError>;

pub(super) fn video_edit(operation: VideoEditOperation) -> VideoEditResult {
    let base = complete_url(operation.api_base.as_deref());
    let provider = operation.provider.as_deref();
    match operation.call {
        VideoEditCall::Edit(call) => edit(*call, base, provider),
        VideoEditCall::Extension(call) => extension(*call, base, provider),
        VideoEditCall::CreateCharacter(call) => create_character(&call, &base),
        VideoEditCall::GetCharacter(call) => get_character(&call, &base),
    }
}

fn edit(call: VideoEditInput, base: String, provider: Option<&str>) -> VideoEditResult {
    let video_file = call.video_file.as_deref().map(MediaBody::Bytes);
    let request = edit_request(VideoEditRequest {
        prompt: call.prompt,
        video_id: call.video_id,
        api_base: base,
        video_file,
        extra_body: call.extra_body,
        prefetched_source_data: None,
    });
    let reply = video_reply(&call.reply, provider)?;
    let VideoEdit { url, body, files } = request;
    let multipart = files.is_some();
    let parts = files.into_iter().map(media_part).collect();
    Ok(upload_outcome(url, body, parts, multipart, reply))
}

fn extension(call: VideoExtensionInput, base: String, provider: Option<&str>) -> VideoEditResult {
    let request = extension_request(VideoExtensionRequest {
        prompt: call.prompt,
        video_id: call.video_id,
        seconds: call.seconds,
        api_base: base,
        extra_body: call.extra_body,
    });
    let reply = video_reply(&call.reply, provider)?;
    let VideoExtension { url, body } = request;
    Ok(upload_outcome(url, body, Vec::new(), false, reply))
}

fn create_character(call: &CharacterCreateInput, base: &str) -> VideoEditResult {
    let created = character_request(&call.name, MediaBody::Bytes(&call.video), base);
    let character = decode_character_reply(&call.reply)?;
    Ok(created_character(created, character))
}

fn get_character(call: &CharacterGetInput, base: &str) -> VideoEditResult {
    let url = character_get_request(&call.character_id, base)?;
    let character = decode_character_reply(&call.reply)?;
    Ok(empty_outcome(
        MediaMethod::Get,
        url,
        VideoEditReplyKind::Character(Box::new(character)),
    ))
}

fn video_reply(body: &[u8], provider: Option<&str>) -> Result<VideoObject, OpenAiMediaError> {
    transform_video_response(&VideoReply {
        body,
        custom_llm_provider: provider,
        request_data: None,
    })
    .map_err(OpenAiMediaError::from)
}

fn upload_outcome(
    url: String,
    body: IndexMap<String, Json>,
    parts: Vec<MediaPart>,
    multipart: bool,
    reply: VideoObject,
) -> VideoEditOutcome {
    VideoEditOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Body(body),
            parts,
            multipart,
        },
        reply: VideoEditReplyKind::Video(Box::new(reply)),
    }
}

fn created_character(created: CreateCharacter<'_>, character: CharacterObject) -> VideoEditOutcome {
    let CreateCharacter { url, files } = created;
    VideoEditOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Empty,
            parts: files.into_iter().map(character_part).collect(),
            multipart: true,
        },
        reply: VideoEditReplyKind::Character(Box::new(character)),
    }
}

fn empty_outcome(method: MediaMethod, url: String, reply: VideoEditReplyKind) -> VideoEditOutcome {
    VideoEditOutcome {
        request: OpenAiMediaPrepared {
            method,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Empty,
            parts: Vec::new(),
            multipart: false,
        },
        reply,
    }
}

// The name field and the video upload stay in the order the character transform built them.
fn character_part(part: CharacterPart<'_>) -> MediaPart {
    match part {
        CharacterPart::Text { field, text } => MediaPart {
            field: field.to_owned(),
            filename: None,
            content_type: None,
            body: MediaPartBody::Text(text.to_owned()),
        },
        CharacterPart::File(file_part) => media_part(file_part),
    }
}
