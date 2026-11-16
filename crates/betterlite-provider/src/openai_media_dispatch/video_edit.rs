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
use super::{
    MediaMethod, MediaPart, MediaPartBody, MediaPayload, OpenAiMediaPrepared, VideoEditOutcome,
    VideoEditReplyKind, empty_body_request,
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
    let provider_name = provider.as_deref();
    match *call {
        VideoEditCall::Edit(edit_call) => edit(&base, provider_name, *edit_call),
        VideoEditCall::Extension(extension_call) => {
            extension(&base, provider_name, *extension_call)
        }
        VideoEditCall::CreateCharacter(character_call) => create_character(&base, *character_call),
        VideoEditCall::GetCharacter(character_call) => get_character(&base, *character_call),
    }
}

fn edit(
    base: &str,
    provider: Option<&str>,
    call: VideoEditInput,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let video_file = call.video_file.as_deref().map(MediaBody::Bytes);
    let request = edit_request(VideoEditRequest {
        prompt: call.prompt,
        video_id: call.video_id,
        api_base: base.to_owned(),
        video_file,
        extra_body: call.extra_body,
        prefetched_source_data: None,
    });
    let reply = transform_video_response(&VideoReply {
        body: &call.reply,
        custom_llm_provider: provider,
        request_data: None,
    })?;
    let VideoEdit { url, body, files } = request;
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
    call: VideoExtensionInput,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let request = extension_request(VideoExtensionRequest {
        prompt: call.prompt,
        video_id: call.video_id,
        seconds: call.seconds,
        api_base: base.to_owned(),
        extra_body: call.extra_body,
    });
    let reply = transform_video_response(&VideoReply {
        body: &call.reply,
        custom_llm_provider: provider,
        request_data: None,
    })?;
    let VideoExtension { url, body } = request;
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
    call: CharacterCreateInput,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let CharacterCreateInput { name, video, reply } = call;
    let created = character_request(&name, MediaBody::Bytes(&video), base);
    let character = decode_character_reply(&reply)?;
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
    call: CharacterGetInput,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let CharacterGetInput {
        character_id,
        reply,
    } = call;
    let url = character_get_request(&character_id, base)?;
    let character = decode_character_reply(&reply)?;
    Ok(VideoEditOutcome {
        request: empty_body_request(MediaMethod::Get, url, VideoOptions::new()),
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
        CharacterPart::File(file_part) => media_part(file_part),
    }
}
