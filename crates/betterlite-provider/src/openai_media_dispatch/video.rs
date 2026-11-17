use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::{MULTIPART_FORM_DATA, complete_url};
use betterlite_openai::videos::content::content_request;
use betterlite_openai::videos::create::{CreateVideoForm, create_request, create_response};
use betterlite_openai::videos::list::{list_request, list_response};
use betterlite_openai::videos::remix::{remix_request, remix_response};
use betterlite_openai::videos::reply::{VideoObject, decode_video};
use betterlite_openai::videos::segment::path_url;
use betterlite_openai::videos::status::status_response;

use super::call::{
    VideoCall, VideoContentCall, VideoCreateCall, VideoDeleteCall, VideoListCall, VideoOperation,
    VideoRemixCall, VideoStatusCall,
};
use super::error::OpenAiMediaError;
use super::parts::media_part;
use super::{MediaMethod, MediaPayload, OpenAiMediaPrepared, VideoOutcome, VideoReplyKind};

type VideoResult = Result<VideoOutcome, OpenAiMediaError>;

pub(super) fn video(operation: VideoOperation) -> VideoResult {
    let base = complete_url(operation.api_base.as_deref());
    let provider = operation.provider.as_deref();
    match operation.call {
        VideoCall::Create(call) => create(&call, &base, provider),
        VideoCall::List(call) => list(&call, &base, provider),
        VideoCall::Status(call) => status(&call, &base, provider),
        VideoCall::Delete(call) => delete(&call, &base),
        VideoCall::Content(call) => content(call, &base),
        VideoCall::Remix(call) => remix(&call, &base, provider),
    }
}

fn create(call: &VideoCreateCall, base: &str, provider: Option<&str>) -> VideoResult {
    let media = call.input_reference.as_deref().map(MediaBody::Bytes);
    let form = create_request(&call.model, &call.prompt, base, &call.options, media);
    let video = create_response(&call.model, &call.reply, provider)?;
    Ok(form_outcome(form, video))
}

fn list(call: &VideoListCall, base: &str, provider: Option<&str>) -> VideoResult {
    let (url, query) = list_request(
        base,
        call.after.as_deref(),
        call.limit,
        call.order.as_deref(),
        &call.extra_query,
    );
    let reply = list_response(&call.reply, provider)?;
    let kind = VideoReplyKind::List(Box::new(reply));
    Ok(video_outcome(
        MediaMethod::Get,
        url,
        query,
        MediaPayload::Empty,
        kind,
    ))
}

fn status(call: &VideoStatusCall, base: &str, provider: Option<&str>) -> VideoResult {
    let url = path_url(&call.video_id, base, "")?;
    let video = status_response(&call.reply, provider)?;
    Ok(path_outcome(MediaMethod::Get, url, video))
}

fn delete(call: &VideoDeleteCall, base: &str) -> VideoResult {
    let url = path_url(&call.video_id, base, "")?;
    let video = decode_video(&call.reply)?;
    Ok(path_outcome(MediaMethod::Delete, url, video))
}

fn content(call: Box<VideoContentCall>, base: &str) -> VideoResult {
    let (url, query) = content_request(&call.video_id, base, call.variant.as_deref())?;
    let kind = VideoReplyKind::Bytes(call.reply);
    Ok(video_outcome(
        MediaMethod::Get,
        url,
        query,
        MediaPayload::Empty,
        kind,
    ))
}

fn remix(call: &VideoRemixCall, base: &str, provider: Option<&str>) -> VideoResult {
    let (url, body) = remix_request(&call.video_id, &call.prompt, base, &call.extra_body)?;
    let reply = remix_response(&call.reply, provider)?;
    let kind = VideoReplyKind::Video(Box::new(reply));
    Ok(post_outcome(url, body, kind))
}

// The remix body is the leaf's JSON options, and the remix asks for no query beside it.
fn post_outcome(url: String, body: VideoOptions, reply: VideoReplyKind) -> VideoOutcome {
    video_outcome(
        MediaMethod::Post,
        url,
        VideoOptions::new(),
        MediaPayload::Options(body),
        reply,
    )
}

// The create form carries its own files, so it is the one outcome that keeps the leaf's parts.
fn form_outcome(form: CreateVideoForm<'_>, video: VideoObject) -> VideoOutcome {
    let CreateVideoForm {
        fields,
        files,
        api_base,
    } = form;
    VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url: api_base,
            query: VideoOptions::new(),
            body: MediaPayload::Options(fields),
            parts: files.into_iter().map(media_part).collect(),
            multipart: MULTIPART_FORM_DATA,
        },
        reply: VideoReplyKind::Video(Box::new(video)),
    }
}

fn video_outcome(
    method: MediaMethod,
    url: String,
    query: VideoOptions,
    body: MediaPayload,
    reply: VideoReplyKind,
) -> VideoOutcome {
    VideoOutcome {
        request: OpenAiMediaPrepared {
            method,
            url,
            query,
            body,
            parts: Vec::new(),
            multipart: false,
        },
        reply,
    }
}

fn path_outcome(method: MediaMethod, url: String, video: VideoObject) -> VideoOutcome {
    video_outcome(
        method,
        url,
        VideoOptions::new(),
        MediaPayload::Empty,
        VideoReplyKind::Video(Box::new(video)),
    )
}
