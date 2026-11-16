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
    let provider_name = provider.as_deref();
    match *call {
        VideoCall::Create(create_call) => create(&base, provider_name, *create_call),
        VideoCall::List(list_call) => list(&base, provider_name, *list_call),
        VideoCall::Status(status_call) => status(&base, provider_name, *status_call),
        VideoCall::Delete(delete_call) => delete(&base, *delete_call),
        VideoCall::Content(content_call) => content(&base, *content_call),
        VideoCall::Remix(remix_call) => remix(&base, provider_name, *remix_call),
    }
}

fn create(
    base: &str,
    provider: Option<&str>,
    call: VideoCreateCall,
) -> Result<VideoOutcome, OpenAiMediaError> {
    let VideoCreateCall {
        model,
        prompt,
        options,
        input_reference,
        reply,
    } = call;
    let media = input_reference.as_deref().map(MediaBody::Bytes);
    let form = create_request(&model, &prompt, base, &options, media);
    let video = create_response(&model, &reply, provider)?;
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
    call: VideoListCall,
) -> Result<VideoOutcome, OpenAiMediaError> {
    let VideoListCall {
        after,
        limit,
        order,
        extra_query,
        reply,
    } = call;
    let (url, query) = list_request(
        base,
        after.as_deref(),
        limit,
        order.as_deref(),
        &extra_query,
    );
    let list = list_response(&reply, provider)?;
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
    call: VideoStatusCall,
) -> Result<VideoOutcome, OpenAiMediaError> {
    let VideoStatusCall { video_id, reply } = call;
    let url = path_url(&video_id, base, "")?;
    let video = status_response(&reply, provider)?;
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Get,
            url,
            query: VideoOptions::new(),
            body: MediaPayload::Empty,
            parts: Vec::new(),
            multipart: false,
        },
        reply: VideoReplyKind::Video(Box::new(video)),
    })
}

fn delete(base: &str, call: VideoDeleteCall) -> Result<VideoOutcome, OpenAiMediaError> {
    let VideoDeleteCall { video_id, reply } = call;
    let url = path_url(&video_id, base, "")?;
    let video = decode_video(&reply)?;
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

fn content(base: &str, call: VideoContentCall) -> Result<VideoOutcome, OpenAiMediaError> {
    let VideoContentCall {
        video_id,
        variant,
        reply,
    } = call;
    let (url, query) = content_request(&video_id, base, variant.as_deref())?;
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Get,
            url,
            query,
            body: MediaPayload::Empty,
            parts: Vec::new(),
            multipart: false,
        },
        reply: VideoReplyKind::Bytes(reply),
    })
}

fn remix(
    base: &str,
    provider: Option<&str>,
    call: VideoRemixCall,
) -> Result<VideoOutcome, OpenAiMediaError> {
    let VideoRemixCall {
        video_id,
        prompt,
        extra_body,
        reply,
    } = call;
    let (url, body) = remix_request(&video_id, &prompt, base, &extra_body)?;
    let video = remix_response(&reply, provider)?;
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
