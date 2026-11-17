use betterlite_openai::openai_video_identifiers::multipart::MediaBody;
use betterlite_openai::videos::VideoOptions;
use betterlite_openai::videos::config::MULTIPART_FORM_DATA;
use betterlite_openai::videos::create::{CreateVideoForm, create_request, create_response};
use betterlite_openai::videos::list::{list_request, list_response};

use super::super::call::{VideoCreateCall, VideoListCall};
use super::super::parts::media_part;
use super::super::{MediaMethod, MediaPayload, OpenAiMediaPrepared, VideoOutcome, VideoReplyKind};
use super::VideoResult;

pub(super) fn create(call: &VideoCreateCall, base: &str, provider: Option<&str>) -> VideoResult {
    let media = call.input_reference.as_deref().map(MediaBody::Bytes);
    let form = create_request(&call.model, &call.prompt, base, &call.options, media);
    let CreateVideoForm {
        fields,
        files,
        api_base,
    } = form;
    let parts = files.into_iter().map(media_part).collect();
    let body = MediaPayload::Options(fields);
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Post,
            url: api_base,
            query: VideoOptions::new(),
            body,
            parts,
            multipart: MULTIPART_FORM_DATA,
        },
        reply: VideoReplyKind::Video(Box::new(create_response(
            &call.model,
            &call.reply,
            provider,
        )?)),
    })
}

pub(super) fn list(call: &VideoListCall, base: &str, provider: Option<&str>) -> VideoResult {
    let (url, query) = list_request(
        base,
        call.after.as_deref(),
        call.limit,
        call.order.as_deref(),
        &call.extra_query,
    );
    let reply = VideoReplyKind::List(Box::new(list_response(&call.reply, provider)?));
    Ok(VideoOutcome {
        request: OpenAiMediaPrepared {
            method: MediaMethod::Get,
            url,
            query,
            body: MediaPayload::Empty,
            parts: Vec::new(),
            multipart: false,
        },
        reply,
    })
}
