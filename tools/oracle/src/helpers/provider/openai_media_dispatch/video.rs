use betterlite_openai::videos::VideoOptions;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_media_dispatch::call::{
    CharacterCreateInput, CharacterGetInput, OpenAiMediaOperation, VideoCall, VideoContentCall,
    VideoCreateCall, VideoDeleteCall, VideoEditCall, VideoEditInput, VideoEditOperation,
    VideoExtensionInput, VideoListCall, VideoOperation, VideoRemixCall, VideoStatusCall,
};
use betterlite_provider::openai_media_dispatch::{
    OpenAiMediaPrepared, OpenAiMediaResult, VideoEditOutcome, VideoEditReplyKind, VideoOutcome,
    VideoReplyKind,
};
use betterlite_provider::operation::OperationResult;
use serde_json::Value;

use super::fixtures::Fixture;
use super::views::{method_name, parts_value, payload_value, query_value};
use super::{OTHER_OUTCOME, VideoCase, VideoCreateCase, VideoOp, failure, leaf_failure, run};

pub(super) async fn video_create(case: &VideoCreateCase) -> HelperResult {
    let call = VideoCreateCall {
        model: case.model.clone(),
        prompt: case.prompt.clone(),
        options: case.options.clone(),
        input_reference: case
            .reference
            .as_ref()
            .map(|reference| reference.as_bytes().to_vec()),
        reply: case.reply.clone().into_bytes(),
    };
    let operation = VideoOperation {
        api_base: Some(case.api_base.clone()),
        provider: case.provider.clone(),
        call: VideoCall::Create(Box::new(call)),
    };
    let OpenAiMediaResult::Video(outcome) =
        media_outcome(OpenAiMediaOperation::Video(Box::new(operation))).await?
    else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    let VideoOutcome { request, reply } = *outcome;
    let VideoReplyKind::Video(video) = reply else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    let video = serde_json::to_value(&video).map_err(|error| failure("Serialize", error))?;
    upload_value(request, video)
}

// The lifecycle and extended cases differ only in which call their op names, so one match builds both.
pub(super) async fn video_case(case: &VideoCase) -> HelperResult {
    let operation = video_operation(case)?;
    match media_outcome(operation).await? {
        OpenAiMediaResult::Video(outcome) => lifecycle_value(*outcome),
        OpenAiMediaResult::VideoEdit(outcome) => edit_value(*outcome),
        _ => Err(failure("Variant", OTHER_OUTCOME)),
    }
}

fn lifecycle_value(outcome: VideoOutcome) -> HelperResult {
    let VideoOutcome { request, reply } = outcome;
    let reply_value = match reply {
        VideoReplyKind::Video(video) => {
            serde_json::to_value(&video).map_err(|error| failure("Serialize", error))?
        }
        VideoReplyKind::List(list) => {
            serde_json::to_value(&list).map_err(|error| failure("Serialize", error))?
        }
        VideoReplyKind::Bytes(bytes) => serde_json::json!({
            "body": String::from_utf8(bytes).map_err(|error| failure("Utf8", error))?
        }),
    };
    Ok(serde_json::json!({
        "request": {
            "method": method_name(request.method),
            "url": request.url,
            "params": query_value(&request.query),
            "body": payload_value(&request.body)?,
            "parts": parts_value(&request.parts)?,
            "multipart": request.multipart,
        },
        "reply": reply_value,
    }))
}

fn edit_value(outcome: VideoEditOutcome) -> HelperResult {
    let VideoEditOutcome { request, reply } = outcome;
    let reply_value = match reply {
        VideoEditReplyKind::Video(video) => {
            serde_json::to_value(&video).map_err(|error| failure("Serialize", error))?
        }
        VideoEditReplyKind::Character(character) => {
            serde_json::to_value(&character).map_err(|error| failure("Serialize", error))?
        }
    };
    upload_value(request, reply_value)
}

enum MediaCall {
    Video(VideoCall),
    Edit(VideoEditCall),
}

fn video_operation(case: &VideoCase) -> Result<OpenAiMediaOperation, Box<HelperError>> {
    let reply = case.reply.clone().into_bytes();
    let call = match case.op {
        VideoOp::List => MediaCall::Video(VideoCall::List(Box::new(VideoListCall {
            after: case.after.clone(),
            limit: case.limit,
            order: case.order.clone(),
            extra_query: VideoOptions::new(),
            reply,
        }))),
        VideoOp::Status => MediaCall::Video(VideoCall::Status(Box::new(VideoStatusCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            reply,
        }))),
        VideoOp::Delete => MediaCall::Video(VideoCall::Delete(Box::new(VideoDeleteCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            reply,
        }))),
        VideoOp::Content => MediaCall::Video(VideoCall::Content(Box::new(VideoContentCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            variant: case.variant.clone(),
            reply,
        }))),
        VideoOp::Remix => MediaCall::Video(VideoCall::Remix(Box::new(VideoRemixCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            prompt: required(&case.prompt, "prompt")?.to_owned(),
            extra_body: VideoOptions::new(),
            reply,
        }))),
        VideoOp::Edit => MediaCall::Edit(VideoEditCall::Edit(Box::new(VideoEditInput {
            prompt: required(&case.prompt, "prompt")?.to_owned(),
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            video_file: case
                .video_file
                .as_ref()
                .map(|video| video.as_bytes().to_vec()),
            extra_body: case.extra_body.clone(),
            reply,
        }))),
        VideoOp::Extension => {
            MediaCall::Edit(VideoEditCall::Extension(Box::new(VideoExtensionInput {
                prompt: required(&case.prompt, "prompt")?.to_owned(),
                video_id: required(&case.video_id, "video_id")?.to_owned(),
                seconds: required(&case.seconds, "seconds")?.to_owned(),
                extra_body: case.extra_body.clone(),
                reply,
            })))
        }
        VideoOp::CreateCharacter => MediaCall::Edit(VideoEditCall::CreateCharacter(Box::new(
            CharacterCreateInput {
                name: required(&case.name, "name")?.to_owned(),
                video: required(&case.video, "video")?.as_bytes().to_vec(),
                reply,
            },
        ))),
        VideoOp::GetCharacter => {
            MediaCall::Edit(VideoEditCall::GetCharacter(Box::new(CharacterGetInput {
                character_id: required(&case.character_id, "character_id")?.to_owned(),
                reply,
            })))
        }
    };
    let api_base = Some(case.api_base.clone());
    let provider = case.provider.clone();
    Ok(match call {
        MediaCall::Video(call) => OpenAiMediaOperation::Video(Box::new(VideoOperation {
            api_base,
            provider,
            call,
        })),
        MediaCall::Edit(call) => OpenAiMediaOperation::VideoEdit(Box::new(VideoEditOperation {
            api_base,
            provider,
            call,
        })),
    })
}

async fn media_outcome(
    operation: OpenAiMediaOperation,
) -> Result<OpenAiMediaResult, Box<HelperError>> {
    let fixture = Fixture::new(200, b"");
    match run(&fixture, operation).await {
        Ok(OperationResult::OpenAiMedia(result)) => Ok(result),
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Err(leaf_failure(&error)),
    }
}

fn upload_value(request: OpenAiMediaPrepared, reply: Value) -> HelperResult {
    Ok(serde_json::json!({
        "url": request.url,
        "body": payload_value(&request.body)?,
        "parts": parts_value(&request.parts)?,
        "multipart": request.multipart,
        "reply": reply,
    }))
}

fn required<'value>(
    field: &'value Option<String>,
    name: &str,
) -> Result<&'value str, Box<HelperError>> {
    field
        .as_deref()
        .ok_or_else(|| failure("Input", format!("the case needs {name}")))
}
