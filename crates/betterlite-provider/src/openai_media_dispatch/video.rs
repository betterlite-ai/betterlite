use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::{MULTIPART_FORM_DATA, complete_url};
use betterlite_openai::videos::content::content_request;
use betterlite_openai::videos::create::{CreateVideoForm, create_request, create_response};
use betterlite_openai::videos::list::{list_request, list_response};
use betterlite_openai::videos::remix::{remix_request, remix_response};
use betterlite_openai::videos::reply::decode_video;
use betterlite_openai::videos::segment::path_url;
use betterlite_openai::videos::status::status_response;

use super::call::{VideoCall, VideoOperation};
use super::error::OpenAiMediaError;
use super::parts::media_part;
use super::{MediaMethod, MediaPayload, OpenAiMediaPrepared, VideoOutcome, VideoReplyKind};

// Every video handler completes the base URL first, then hands it to the leaf's own request transform.
pub(super) fn video(operation: VideoOperation) -> Result<VideoOutcome, OpenAiMediaError> {
    let base = complete_url(operation.api_base.as_deref());
    let provider_name = operation.provider.as_deref();
    let request = video_request(&operation.call, &base)?;
    let reply = match operation.call {
        VideoCall::Create(call) => {
            let video = create_response(&call.model, &call.reply, provider_name)?;
            VideoReplyKind::Video(Box::new(video))
        }
        VideoCall::List(call) => {
            VideoReplyKind::List(Box::new(list_response(&call.reply, provider_name)?))
        }
        VideoCall::Status(call) => {
            VideoReplyKind::Video(Box::new(status_response(&call.reply, provider_name)?))
        }
        VideoCall::Delete(call) => VideoReplyKind::Video(Box::new(decode_video(&call.reply)?)),
        VideoCall::Content(call) => VideoReplyKind::Bytes(call.reply),
        VideoCall::Remix(call) => {
            VideoReplyKind::Video(Box::new(remix_response(&call.reply, provider_name)?))
        }
    };
    Ok(VideoOutcome { request, reply })
}

fn video_request(call: &VideoCall, base: &str) -> Result<OpenAiMediaPrepared, OpenAiMediaError> {
    match call {
        VideoCall::Create(call) => {
            let media = call.input_reference.as_deref().map(MediaBody::Bytes);
            let form = create_request(&call.model, &call.prompt, base, &call.options, media);
            let CreateVideoForm {
                fields,
                files,
                api_base,
            } = form;
            Ok(OpenAiMediaPrepared {
                method: MediaMethod::Post,
                url: api_base,
                query: VideoOptions::new(),
                body: MediaPayload::Options(fields),
                parts: files.into_iter().map(media_part).collect(),
                multipart: MULTIPART_FORM_DATA,
            })
        }
        VideoCall::List(call) => {
            let (url, query) = list_request(
                base,
                call.after.as_deref(),
                call.limit,
                call.order.as_deref(),
                &call.extra_query,
            );
            Ok(OpenAiMediaPrepared {
                method: MediaMethod::Get,
                url,
                query,
                body: MediaPayload::Empty,
                parts: Vec::new(),
                multipart: false,
            })
        }
        VideoCall::Status(call) => {
            let url = path_url(&call.video_id, base, "")?;
            Ok(OpenAiMediaPrepared {
                method: MediaMethod::Get,
                url,
                query: VideoOptions::new(),
                body: MediaPayload::Empty,
                parts: Vec::new(),
                multipart: false,
            })
        }
        VideoCall::Delete(call) => {
            let url = path_url(&call.video_id, base, "")?;
            Ok(OpenAiMediaPrepared {
                method: MediaMethod::Delete,
                url,
                query: VideoOptions::new(),
                body: MediaPayload::Empty,
                parts: Vec::new(),
                multipart: false,
            })
        }
        VideoCall::Content(call) => {
            let (url, query) = content_request(&call.video_id, base, call.variant.as_deref())?;
            Ok(OpenAiMediaPrepared {
                method: MediaMethod::Get,
                url,
                query,
                body: MediaPayload::Empty,
                parts: Vec::new(),
                multipart: false,
            })
        }
        VideoCall::Remix(call) => {
            let (url, body) = remix_request(&call.video_id, &call.prompt, base, &call.extra_body)?;
            Ok(OpenAiMediaPrepared {
                method: MediaMethod::Post,
                url,
                query: VideoOptions::new(),
                body: MediaPayload::Options(body),
                parts: Vec::new(),
                multipart: false,
            })
        }
    }
}
