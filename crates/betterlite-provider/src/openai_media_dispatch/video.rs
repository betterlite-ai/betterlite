use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::complete_url;
use betterlite_openai::videos::content::content_request;
use betterlite_openai::videos::remix::{remix_request, remix_response};
use betterlite_openai::videos::reply::decode_video;
use betterlite_openai::videos::segment::path_url;
use betterlite_openai::videos::status::status_response;

use super::call::{
    VideoCall, VideoContentCall, VideoDeleteCall, VideoOperation, VideoRemixCall, VideoStatusCall,
};
use super::error::OpenAiMediaError;
use super::{MediaMethod, MediaPayload, OpenAiMediaPrepared, VideoOutcome, VideoReplyKind};

mod collection;

type VideoResult = Result<VideoOutcome, OpenAiMediaError>;

pub(super) fn video(operation: VideoOperation) -> VideoResult {
    let base = complete_url(operation.api_base.as_deref());
    let provider = operation.provider.as_deref();
    match operation.call {
        VideoCall::Create(call) => collection::create(&call, &base, provider),
        VideoCall::List(call) => collection::list(&call, &base, provider),
        VideoCall::Status(call) => status(&call, &base, provider),
        VideoCall::Delete(call) => delete(&call, &base),
        VideoCall::Content(call) => content(*call, &base),
        VideoCall::Remix(call) => remix(&call, &base, provider),
    }
}

fn status(call: &VideoStatusCall, base: &str, provider: Option<&str>) -> VideoResult {
    let VideoStatusCall {
        video_id,
        reply: body,
    } = call;
    let url = path_url(video_id, base, "")?;
    let video = status_response(body, provider)?;
    let reply = VideoReplyKind::Video(Box::new(video));
    let query = VideoOptions::new();
    let request = OpenAiMediaPrepared {
        method: MediaMethod::Get,
        url,
        query,
        body: MediaPayload::Empty,
        parts: Vec::new(),
        multipart: false,
    };
    Ok(VideoOutcome { request, reply })
}

fn delete(call: &VideoDeleteCall, base: &str) -> VideoResult {
    let url = path_url(&call.video_id, base, "")?;
    let video = decode_video(&call.reply)?;
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Delete,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Empty,
            parts: Vec::new(),
            multipart: false,
        },
        reply: VideoReplyKind::Video(Box::new(video)),
    })
}

fn content(call: VideoContentCall, base: &str) -> VideoResult {
    let (url, query) = content_request(&call.video_id, base, call.variant.as_deref())?;
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Get,
            url,
            query,
            body: MediaPayload::Empty,
            parts: Vec::new(),
            multipart: false,
        },
        reply: VideoReplyKind::Bytes(call.reply),
    })
}

fn remix(call: &VideoRemixCall, base: &str, provider: Option<&str>) -> VideoResult {
    let (url, body) = remix_request(&call.video_id, &call.prompt, base, &call.extra_body)?;
    let request = OpenAiMediaPrepared {
        method: MediaMethod::Post,
        url,
        query: VideoOptions::new(),
        body: MediaPayload::Options(body),
        parts: Vec::new(),
        multipart: false,
    };
    Ok(VideoOutcome {
        request,
        reply: VideoReplyKind::Video(Box::new(remix_response(&call.reply, provider)?)),
    })
}
