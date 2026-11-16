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

use super::call::{VideoEditCall, VideoEditOperation};
use super::error::OpenAiMediaError;
use super::parts::media_part;
use super::{
    MediaMethod, MediaPart, MediaPartBody, MediaPayload, OpenAiMediaPrepared, VideoEditOutcome,
    VideoEditReplyKind,
};

pub(super) fn video_edit(
    operation: VideoEditOperation,
) -> Result<VideoEditOutcome, OpenAiMediaError> {
    let base = complete_url(operation.api_base.as_deref());
    let provider_name = operation.provider.as_deref();
    let (request, reply) = match operation.call {
        VideoEditCall::Edit(call) => {
            let video_file = call.video_file.as_deref().map(MediaBody::Bytes);
            let request = edit_request(VideoEditRequest {
                prompt: call.prompt,
                video_id: call.video_id,
                api_base: base,
                video_file,
                extra_body: call.extra_body,
                prefetched_source_data: None,
            });
            let reply = transform_video_response(&VideoReply {
                body: &call.reply,
                custom_llm_provider: provider_name,
                request_data: None,
            })?;
            let VideoEdit { url, body, files } = request;
            let multipart = files.is_some();
            (
                OpenAiMediaPrepared {
                    method: MediaMethod::Post,
                    url,
                    query: VideoOptions::new(),
                    body: MediaPayload::Body(body),
                    parts: files.into_iter().map(media_part).collect(),
                    multipart,
                },
                VideoEditReplyKind::Video(Box::new(reply)),
            )
        }
        VideoEditCall::Extension(call) => {
            let request = extension_request(VideoExtensionRequest {
                prompt: call.prompt,
                video_id: call.video_id,
                seconds: call.seconds,
                api_base: base,
                extra_body: call.extra_body,
            });
            let reply = transform_video_response(&VideoReply {
                body: &call.reply,
                custom_llm_provider: provider_name,
                request_data: None,
            })?;
            let VideoExtension { url, body } = request;
            (
                OpenAiMediaPrepared {
                    method: MediaMethod::Post,
                    url,
                    query: VideoOptions::new(),
                    body: MediaPayload::Body(body),
                    parts: Vec::new(),
                    multipart: false,
                },
                VideoEditReplyKind::Video(Box::new(reply)),
            )
        }
        VideoEditCall::CreateCharacter(call) => {
            let created = character_request(&call.name, MediaBody::Bytes(&call.video), &base);
            let character = decode_character_reply(&call.reply)?;
            let CreateCharacter { url, files } = created;
            (
                OpenAiMediaPrepared {
                    method: MediaMethod::Post,
                    url,
                    query: VideoOptions::new(),
                    body: MediaPayload::Empty,
                    parts: files.into_iter().map(character_part).collect(),
                    multipart: true,
                },
                VideoEditReplyKind::Character(Box::new(character)),
            )
        }
        VideoEditCall::GetCharacter(call) => {
            let url = character_get_request(&call.character_id, &base)?;
            let character = decode_character_reply(&call.reply)?;
            (
                OpenAiMediaPrepared {
                    method: MediaMethod::Get,
                    url,
                    query: VideoOptions::new(),
                    body: MediaPayload::Empty,
                    parts: Vec::new(),
                    multipart: false,
                },
                VideoEditReplyKind::Character(Box::new(character)),
            )
        }
    };
    Ok(VideoEditOutcome { request, reply })
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
