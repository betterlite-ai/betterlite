use betterlite_openai::videos::VideoOptions;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_media_dispatch::call::{
    CharacterCreateInput, CharacterGetInput, OpenAiMediaOperation, VideoCall, VideoContentCall,
    VideoCreateCall, VideoDeleteCall, VideoEditCall, VideoEditInput, VideoEditOperation,
    VideoExtensionInput, VideoListCall, VideoOperation, VideoRemixCall, VideoStatusCall,
};
use betterlite_provider::openai_media_dispatch::error::OpenAiMediaError;
use betterlite_provider::openai_media_dispatch::execute::dispatch;
use betterlite_provider::openai_media_dispatch::{
    OpenAiMediaRequest, OpenAiMediaResult, VideoEditOutcome, VideoEditReplyKind, VideoOutcome,
    VideoReplyKind,
};
use betterlite_provider::operation::{OperationRequest, OperationResult};

use super::fixtures::{Fixture, Recorder};
use super::views::{method_name, parts_value, payload_value, query_value};
use super::{
    OTHER_OUTCOME, VideoCreateCase, VideoExtendedCase, VideoExtendedOp, VideoLifecycleCase,
    VideoLifecycleOp, failure, leaf_failure,
};

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
    let outcome = match run(OpenAiMediaOperation::Video(Box::new(operation))).await {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::Video(outcome))) => outcome,
        Ok(_) => return Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => return Err(leaf_failure(&error)),
    };
    let VideoOutcome { request, reply } = outcome;
    let VideoReplyKind::Video(video) = reply else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    Ok(serde_json::json!({
        "url": request.url,
        "body": payload_value(&request.body)?,
        "parts": parts_value(&request.parts)?,
        "multipart": request.multipart,
        "reply": serde_json::to_value(&video).map_err(|error| failure("Serialize", error))?,
    }))
}

pub(super) async fn video_lifecycle(case: &VideoLifecycleCase) -> HelperResult {
    let operation = VideoOperation {
        api_base: Some(case.api_base.clone()),
        provider: case.provider.clone(),
        call: lifecycle_call(case)?,
    };
    let outcome = match run(OpenAiMediaOperation::Video(Box::new(operation))).await {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::Video(outcome))) => outcome,
        Ok(_) => return Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => return Err(leaf_failure(&error)),
    };
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

pub(super) async fn video_extended(case: &VideoExtendedCase) -> HelperResult {
    let operation = VideoEditOperation {
        api_base: Some(case.api_base.clone()),
        provider: case.provider.clone(),
        call: extended_call(case)?,
    };
    let outcome = match run(OpenAiMediaOperation::VideoEdit(Box::new(operation))).await {
        Ok(OperationResult::OpenAiMedia(OpenAiMediaResult::VideoEdit(outcome))) => outcome,
        Ok(_) => return Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => return Err(leaf_failure(&error)),
    };
    let VideoEditOutcome { request, reply } = outcome;
    let reply_value = match reply {
        VideoEditReplyKind::Video(video) => {
            serde_json::to_value(&video).map_err(|error| failure("Serialize", error))?
        }
        VideoEditReplyKind::Character(character) => {
            serde_json::to_value(&character).map_err(|error| failure("Serialize", error))?
        }
    };
    Ok(serde_json::json!({
        "url": request.url,
        "body": payload_value(&request.body)?,
        "parts": parts_value(&request.parts)?,
        "multipart": request.multipart,
        "reply": reply_value,
    }))
}

async fn run(operation: OpenAiMediaOperation) -> Result<OperationResult, OpenAiMediaError> {
    let fixture = Fixture::new(200, b"");
    let mut logger = Recorder;
    dispatch(
        OperationRequest::OpenAiMedia(OpenAiMediaRequest),
        operation,
        &fixture,
        &mut logger,
    )
    .await
}

fn lifecycle_call(case: &VideoLifecycleCase) -> Result<VideoCall, Box<HelperError>> {
    let reply = case.reply.clone().into_bytes();
    let call = match case.op {
        VideoLifecycleOp::List => VideoCall::List(Box::new(VideoListCall {
            after: case.after.clone(),
            limit: case.limit,
            order: case.order.clone(),
            extra_query: VideoOptions::new(),
            reply,
        })),
        VideoLifecycleOp::Status => VideoCall::Status(Box::new(VideoStatusCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            reply,
        })),
        VideoLifecycleOp::Delete => VideoCall::Delete(Box::new(VideoDeleteCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            reply,
        })),
        VideoLifecycleOp::Content => VideoCall::Content(Box::new(VideoContentCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            variant: case.variant.clone(),
            reply,
        })),
        VideoLifecycleOp::Remix => VideoCall::Remix(Box::new(VideoRemixCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            prompt: required(&case.prompt, "prompt")?.to_owned(),
            extra_body: VideoOptions::new(),
            reply,
        })),
    };
    Ok(call)
}

fn extended_call(case: &VideoExtendedCase) -> Result<VideoEditCall, Box<HelperError>> {
    let reply = case.reply.clone().into_bytes();
    let call = match case.op {
        VideoExtendedOp::Edit => VideoEditCall::Edit(Box::new(VideoEditInput {
            prompt: required(&case.prompt, "prompt")?.to_owned(),
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            video_file: case
                .video_file
                .as_ref()
                .map(|video| video.as_bytes().to_vec()),
            extra_body: case.extra_body.clone(),
            reply,
        })),
        VideoExtendedOp::Extension => VideoEditCall::Extension(Box::new(VideoExtensionInput {
            prompt: required(&case.prompt, "prompt")?.to_owned(),
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            seconds: required(&case.seconds, "seconds")?.to_owned(),
            extra_body: case.extra_body.clone(),
            reply,
        })),
        VideoExtendedOp::CreateCharacter => {
            VideoEditCall::CreateCharacter(Box::new(CharacterCreateInput {
                name: required(&case.name, "name")?.to_owned(),
                video: required(&case.video, "video")?.as_bytes().to_vec(),
                reply,
            }))
        }
        VideoExtendedOp::GetCharacter => VideoEditCall::GetCharacter(Box::new(CharacterGetInput {
            character_id: required(&case.character_id, "character_id")?.to_owned(),
            reply,
        })),
    };
    Ok(call)
}

fn required<'value>(
    field: &'value Option<String>,
    name: &str,
) -> Result<&'value str, Box<HelperError>> {
    field
        .as_deref()
        .ok_or_else(|| failure("Input", format!("the case needs {name}")))
}
