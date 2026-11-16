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

use super::call::{
    VideoCall, VideoContentCall, VideoCreateCall, VideoDeleteCall, VideoListCall, VideoOperation,
    VideoRemixCall, VideoStatusCall,
};
use super::error::OpenAiMediaError;
use super::parts::media_part;
use super::{MediaMethod, MediaPayload, OpenAiMediaPrepared, VideoOutcome, VideoReplyKind};

// Every video handler completes the base URL first, then hands it to the leaf's own request transform.
pub(super) fn video(operation: VideoOperation) -> Result<VideoOutcome, OpenAiMediaError> {
    let VideoOperation {
        api_base,
        provider,
        call,
    } = operation;
    let base = complete_url(api_base.as_deref());
    let provider = provider.as_deref();
    match *call {
        VideoCall::Create(call) => create(&base, provider, &call),
        VideoCall::List(call) => list(&base, provider, &call),
        VideoCall::Status(call) => status(&base, provider, &call),
        VideoCall::Delete(call) => delete(&base, &call),
        VideoCall::Content(call) => content(&base, &call),
        VideoCall::Remix(call) => remix(&base, provider, &call),
    }
}

fn create(
    base: &str,
    provider: Option<&str>,
    call: &VideoCreateCall,
) -> Result<VideoOutcome, OpenAiMediaError> {
    let media = call.input_reference.as_deref().map(MediaBody::Bytes);
    let form = create_request(&call.model, &call.prompt, base, &call.options, media);
    let video = create_response(&call.model, &call.reply, provider)?;
    let CreateVideoForm {
        fields,
        files,
        api_base,
    } = form;
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url: api_base,
            query: VideoOptions::new(),
            body: MediaPayload::Options(fields),
            parts: files.into_iter().map(media_part).collect(),
            multipart: MULTIPART_FORM_DATA,
        },
        reply: VideoReplyKind::Video(Box::new(video)),
    })
}

fn list(
    base: &str,
    provider: Option<&str>,
    call: &VideoListCall,
) -> Result<VideoOutcome, OpenAiMediaError> {
    let (url, query) = list_request(
        base,
        call.after.as_deref(),
        call.limit,
        call.order.as_deref(),
        &call.extra_query,
    );
    let list = list_response(&call.reply, provider)?;
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Get,
            url,
            query,
            body: MediaPayload::Empty,
            parts: Vec::new(),
            multipart: false,
        },
        reply: VideoReplyKind::List(Box::new(list)),
    })
}

fn status(
    base: &str,
    provider: Option<&str>,
    call: &VideoStatusCall,
) -> Result<VideoOutcome, OpenAiMediaError> {
    let url = path_url(&call.video_id, base, "")?;
    let video = status_response(&call.reply, provider)?;
    Ok(VideoOutcome {
        request: video_request(MediaMethod::Get, url),
        reply: VideoReplyKind::Video(Box::new(video)),
    })
}

fn delete(base: &str, call: &VideoDeleteCall) -> Result<VideoOutcome, OpenAiMediaError> {
    let url = path_url(&call.video_id, base, "")?;
    let video = decode_video(&call.reply)?;
    Ok(VideoOutcome {
        request: video_request(MediaMethod::Delete, url),
        reply: VideoReplyKind::Video(Box::new(video)),
    })
}

fn content(base: &str, call: &VideoContentCall) -> Result<VideoOutcome, OpenAiMediaError> {
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
        reply: VideoReplyKind::Bytes(call.reply.clone()),
    })
}

fn remix(
    base: &str,
    provider: Option<&str>,
    call: &VideoRemixCall,
) -> Result<VideoOutcome, OpenAiMediaError> {
    let (url, body) = remix_request(&call.video_id, &call.prompt, base, &call.extra_body)?;
    let video = remix_response(&call.reply, provider)?;
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Options(body),
            parts: Vec::new(),
            multipart: false,
        },
        reply: VideoReplyKind::Video(Box::new(video)),
    })
}

// The body-less video requests carry no query, no parts and no multipart policy.
pub(super) fn video_request(method: MediaMethod, url: String) -> OpenAiMediaPrepared {
    OpenAiMediaPrepared {
        method,
        url,
        query: VideoOptions::new(),
        body: MediaPayload::Empty,
        parts: Vec::new(),
        multipart: false,
    }
}
