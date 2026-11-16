use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::video_edits::character::{
    CharacterPart, CreateCharacter, transform_video_create_character_request,
    transform_video_get_character_request,
};
use betterlite_openai::video_edits::edit::{
    VideoEdit, VideoEditRequest, transform_video_edit_request,
};
use betterlite_openai::video_edits::extension::{
    VideoExtension, VideoExtensionRequest, transform_video_extension_request,
};
use betterlite_openai::video_edits::response::{
    VideoReply, decode_character_reply, transform_video_response,
};
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::complete_url;

use super::call::{
    CharacterCreateInput, CharacterGetInput, VideoEditCall, VideoEditInput, VideoEditOperation,
    VideoExtensionInput,
};
use super::error::OpenAiMediaError;
use super::parts::media_part;
use super::video::video_request;
use super::{
    MediaMethod, MediaPart, MediaPartBody, MediaPayload, OpenAiMediaPrepared, VideoEditOutcome,
    VideoEditReplyKind,
};

pub(super) fn video_edit(
    operation: VideoEditOperation,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let VideoEditOperation {
        api_base,
        provider,
        call,
    } = operation;
    let base = complete_url(api_base.as_deref());
    let provider = provider.as_deref();
    match *call {
        VideoEditCall::Edit(call) => edit(&base, provider, &call),
        VideoEditCall::Extension(call) => extension(&base, provider, &call),
        VideoEditCall::CreateCharacter(call) => create_character(&base, &call),
        VideoEditCall::GetCharacter(call) => get_character(&base, &call),
    }
}

fn edit(
    base: &str,
    provider: Option<&str>,
    call: &VideoEditInput,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let video_file = call.video_file.as_deref().map(MediaBody::Bytes);
    let edit = transform_video_edit_request(VideoEditRequest {
        prompt: call.prompt.clone(),
        video_id: call.video_id.clone(),
        api_base: base.to_owned(),
        video_file,
        extra_body: call.extra_body.clone(),
        prefetched_source_data: None,
    });
    let reply = transform_video_response(&VideoReply {
        body: &call.reply,
        custom_llm_provider: provider,
        request_data: None,
    })?;
    let VideoEdit { url, body, files } = edit;
    let multipart = files.is_some();
    Ok(VideoEditOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Body(body),
            parts: files.into_iter().map(media_part).collect(),
            multipart,
        },
        reply: VideoEditReplyKind::Video(Box::new(reply)),
    })
}

fn extension(
    base: &str,
    provider: Option<&str>,
    call: &VideoExtensionInput,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let extension = transform_video_extension_request(VideoExtensionRequest {
        prompt: call.prompt.clone(),
        video_id: call.video_id.clone(),
        seconds: call.seconds.clone(),
        api_base: base.to_owned(),
        extra_body: call.extra_body.clone(),
    });
    let reply = transform_video_response(&VideoReply {
        body: &call.reply,
        custom_llm_provider: provider,
        request_data: None,
    })?;
    let VideoExtension { url, body } = extension;
    Ok(VideoEditOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Body(body),
            parts: Vec::new(),
            multipart: false,
        },
        reply: VideoEditReplyKind::Video(Box::new(reply)),
    })
}

fn create_character(
    base: &str,
    call: &CharacterCreateInput,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let created =
        transform_video_create_character_request(&call.name, MediaBody::Bytes(&call.video), base);
    let character = decode_character_reply(&call.reply)?;
    let CreateCharacter { url, files } = created;
    Ok(VideoEditOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Empty,
            parts: files.into_iter().map(character_part).collect(),
            multipart: true,
        },
        reply: VideoEditReplyKind::Character(Box::new(character)),
    })
}

fn get_character(
    base: &str,
    call: &CharacterGetInput,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let url = transform_video_get_character_request(&call.character_id, base)?;
    let character = decode_character_reply(&call.reply)?;
    Ok(VideoEditOutcome {
        request: video_request(MediaMethod::Get, url),
        reply: VideoEditReplyKind::Character(Box::new(character)),
    })
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
        CharacterPart::File(part) => media_part(part),
    }
}
