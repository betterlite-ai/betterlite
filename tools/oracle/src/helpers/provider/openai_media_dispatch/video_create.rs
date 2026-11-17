use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::openai_media_dispatch::call::{
    OpenAiMediaOperation, VideoCall, VideoCreateCall, VideoOperation,
};
use betterlite_provider::openai_media_dispatch::{OpenAiMediaResult, VideoOutcome, VideoReplyKind};

use super::video::{media_outcome, upload_value};
use super::{OTHER_OUTCOME, VideoCreateCase, failure};

// The create form is the one video call whose reply the adapter projects as url, body, parts and multipart.
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
    let payload = serde_json::to_value(&video).map_err(|error| failure("Serialize", error))?;
    upload_value(&request, &payload)
}
