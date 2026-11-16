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
    MediaMethod, OpenAiMediaRequest, OpenAiMediaResult, VideoEditOutcome, VideoEditReplyKind,
    VideoOutcome, VideoReplyKind,
};
use betterlite_provider::operation::{OperationRequest, OperationResult};

use super::fixtures::{Fixture, Recorder};
use super::views::{method_name, parts_value, payload_value, query_value, serialized, text};
use super::{
    OTHER_OUTCOME, VideoCreateCase, VideoExtendedCase, VideoLifecycleCase, failure, leaf_failure,
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
        "reply": serialized(&video)?,
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
    let reply = match reply {
        VideoReplyKind::Video(video) => serialized(&video)?,
        VideoReplyKind::List(list) => serialized(&list)?,
        VideoReplyKind::Bytes(bytes) => serde_json::json!({"body": text(&bytes)?}),
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
        "reply": reply,
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
    let reply = match reply {
        VideoEditReplyKind::Video(video) => serialized(&video)?,
        VideoEditReplyKind::Character(character) => serialized(&character)?,
    };
    Ok(serde_json::json!({
        "url": request.url,
        "body": payload_value(&request.body)?,
        "parts": parts_value(&request.parts)?,
        "multipart": request.multipart,
        "reply": reply,
    }))
}

async fn run(operation: OpenAiMediaOperation) -> Result<OperationResult, OpenAiMediaError> {
    let fixture = Fixture::new(200, Vec::new());
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
    let call = match case.op.as_str() {
        "list" => VideoCall::List(Box::new(VideoListCall {
            after: case.after.clone(),
            limit: case.limit,
            order: case.order.clone(),
            extra_query: VideoOptions::new(),
            reply,
        })),
        "status" => VideoCall::Status(Box::new(VideoStatusCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            reply,
        })),
        "delete" => VideoCall::Delete(Box::new(VideoDeleteCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            reply,
        })),
        "content" => VideoCall::Content(Box::new(VideoContentCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            variant: case.variant.clone(),
            reply,
        })),
        "remix" => VideoCall::Remix(Box::new(VideoRemixCall {
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            prompt: required(&case.prompt, "prompt")?.to_owned(),
            extra_body: VideoOptions::new(),
            reply,
        })),
        other => return Err(failure("Input", format!("unknown video op {other}"))),
    };
    Ok(call)
}

fn extended_call(case: &VideoExtendedCase) -> Result<VideoEditCall, Box<HelperError>> {
    let reply = case.reply.clone().into_bytes();
    let call = match case.op.as_str() {
        "edit" => VideoEditCall::Edit(Box::new(VideoEditInput {
            prompt: required(&case.prompt, "prompt")?.to_owned(),
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            video_file: case
                .video_file
                .as_ref()
                .map(|video| video.as_bytes().to_vec()),
            extra_body: case.extra_body.clone(),
            reply,
        })),
        "extension" => VideoEditCall::Extension(Box::new(VideoExtensionInput {
            prompt: required(&case.prompt, "prompt")?.to_owned(),
            video_id: required(&case.video_id, "video_id")?.to_owned(),
            seconds: required(&case.seconds, "seconds")?.to_owned(),
            extra_body: case.extra_body.clone(),
            reply,
        })),
        "create_character" => VideoEditCall::CreateCharacter(Box::new(CharacterCreateInput {
            name: required(&case.name, "name")?.to_owned(),
            video: required(&case.video, "video")?.as_bytes().to_vec(),
            reply,
        })),
        "get_character" => VideoEditCall::GetCharacter(Box::new(CharacterGetInput {
            character_id: required(&case.character_id, "character_id")?.to_owned(),
            reply,
        })),
        other => return Err(failure("Input", format!("unknown video edit op {other}"))),
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
