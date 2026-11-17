use betterlite_openai::videos::VideoOptions;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_media_dispatch::call::{
    OpenAiMediaOperation, VideoCall, VideoContentCall, VideoDeleteCall, VideoEditOperation,
    VideoListCall, VideoOperation, VideoRemixCall, VideoStatusCall,
};
use betterlite_provider::openai_media_dispatch::{
    OpenAiMediaPrepared, OpenAiMediaResult, VideoEditOutcome, VideoEditReplyKind, VideoOutcome,
    VideoReplyKind,
};
use betterlite_provider::operation::OperationResult;
use serde_json::Value;

use super::fixtures::Fixture;
use super::video_extended::extended_call;
use super::views::{method_name, parts_value, payload_value};
use super::{OTHER_OUTCOME, VideoCase, VideoOp, failure, leaf_failure, run};

pub(super) async fn video_case(case: &VideoCase) -> HelperResult {
    let reply = case.reply.clone().into_bytes();
    let api_base = Some(case.api_base.clone());
    let provider = case.provider.clone();
    let operation = match case.op {
        VideoOp::List | VideoOp::Status | VideoOp::Delete | VideoOp::Content | VideoOp::Remix => {
            OpenAiMediaOperation::Video(Box::new(VideoOperation {
                api_base,
                provider,
                call: lifecycle_call(case, reply)?,
            }))
        }
        VideoOp::Edit | VideoOp::Extension | VideoOp::CreateCharacter | VideoOp::GetCharacter => {
            OpenAiMediaOperation::VideoEdit(Box::new(VideoEditOperation {
                api_base,
                provider,
                call: extended_call(case, reply)?,
            }))
        }
    };
    let result = media_outcome(operation).await?;
    if let OpenAiMediaResult::Video(outcome) = result {
        return lifecycle_value(*outcome);
    }
    let OpenAiMediaResult::VideoEdit(outcome) = result else {
        return Err(failure("Variant", OTHER_OUTCOME));
    };
    edit_value(*outcome)
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
            "params": request.query,
            "body": payload_value(&request.body)?,
            "parts": parts_value(&request.parts)?,
            "multipart": request.multipart,
        },
        "reply": reply_value,
    }))
}

fn edit_value(outcome: VideoEditOutcome) -> HelperResult {
    let VideoEditOutcome { request, reply } = outcome;
    let reply_value = edit_reply(reply)?;
    upload_value(&request, &reply_value)
}

fn edit_reply(reply: VideoEditReplyKind) -> Result<Value, Box<HelperError>> {
    match reply {
        VideoEditReplyKind::Video(video) => serde_json::to_value(&video),
        VideoEditReplyKind::Character(character) => serde_json::to_value(&character),
    }
    .map_err(|error| failure("Serialize", error))
}

fn lifecycle_call(case: &VideoCase, reply: Vec<u8>) -> Result<VideoCall, Box<HelperError>> {
    match case.op {
        VideoOp::List => Ok(VideoCall::List(Box::new(VideoListCall {
            after: case.after.clone(),
            limit: case.limit,
            order: case.order.clone(),
            extra_query: VideoOptions::new(),
            reply,
        }))),
        VideoOp::Status => Ok(VideoCall::Status(Box::new(VideoStatusCall {
            video_id: required(case.video_id.as_deref(), "video_id")?.to_owned(),
            reply,
        }))),
        VideoOp::Delete => Ok(VideoCall::Delete(Box::new(VideoDeleteCall {
            video_id: required(case.video_id.as_deref(), "video_id")?.to_owned(),
            reply,
        }))),
        VideoOp::Content => Ok(VideoCall::Content(Box::new(VideoContentCall {
            video_id: required(case.video_id.as_deref(), "video_id")?.to_owned(),
            variant: case.variant.clone(),
            reply,
        }))),
        VideoOp::Remix => Ok(VideoCall::Remix(Box::new(VideoRemixCall {
            video_id: required(case.video_id.as_deref(), "video_id")?.to_owned(),
            prompt: required(case.prompt.as_deref(), "prompt")?.to_owned(),
            extra_body: VideoOptions::new(),
            reply,
        }))),
        VideoOp::Edit | VideoOp::Extension | VideoOp::CreateCharacter | VideoOp::GetCharacter => {
            Err(failure("Variant", OTHER_OUTCOME))
        }
    }
}

pub(super) async fn media_outcome(
    operation: OpenAiMediaOperation,
) -> Result<OpenAiMediaResult, Box<HelperError>> {
    let fixture = Fixture::new(200, b"");
    match run(&fixture, operation).await {
        Ok(OperationResult::OpenAiMedia(result)) => Ok(result),
        Ok(_) => Err(failure("Variant", OTHER_OUTCOME)),
        Err(error) => Err(leaf_failure(&error)),
    }
}

pub(super) fn upload_value(request: &OpenAiMediaPrepared, reply: &Value) -> HelperResult {
    Ok(serde_json::json!({
        "url": request.url,
        "body": payload_value(&request.body)?,
        "parts": parts_value(&request.parts)?,
        "multipart": request.multipart,
        "reply": reply,
    }))
}

pub(super) fn required<'value>(
    field: Option<&'value str>,
    name: &str,
) -> Result<&'value str, Box<HelperError>> {
    let Some(value) = field else {
        return Err(failure("Input", format!("the case needs {name}")));
    };
    Ok(value)
}
