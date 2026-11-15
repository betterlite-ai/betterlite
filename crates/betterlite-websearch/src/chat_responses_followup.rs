pub mod chat;
pub mod error;
pub mod rerun;
pub mod responses;
pub mod search;

mod tests;

use std::sync::Arc;

use betterlite_context::agentic_loop_plan::{AgenticLoopPlan, AgenticLoopRequestPatch, JsonMap};
use serde_json::value::to_raw_value;

use self::chat::FollowupRequest;
use self::error::FollowupError;
use self::rerun::ChatRerunRequest;
use self::responses::ResponsesRequest;
use self::search::SearchExecutor;
use crate::call_transformation::wire::Call;

pub struct ChatResponsesFollowup {
    pub search: Arc<dyn SearchExecutor>,
}

impl ChatResponsesFollowup {
    pub async fn build_chat_plan(
        &self,
        calls: &[Call],
        format: &str,
        followup: &FollowupRequest<'_>,
    ) -> Result<AgenticLoopPlan, FollowupError> {
        websearch_plan(
            chat::build_chat_patch(self.search.as_ref(), calls, format, followup).await?,
            format,
        )
    }

    pub async fn build_responses_plan(
        &self,
        tool_calls: &[Call],
        request: &ResponsesRequest<'_>,
    ) -> Result<AgenticLoopPlan, FollowupError> {
        websearch_plan(
            responses::build_responses_patch(self.search.as_ref(), tool_calls, request).await?,
            "responses",
        )
    }

    pub async fn build_chat_rerun(
        &self,
        tool_calls: &[Call],
        response_format: &str,
        request: &FollowupRequest<'_>,
    ) -> Result<ChatRerunRequest, FollowupError> {
        let patch =
            chat::build_chat_patch(self.search.as_ref(), tool_calls, response_format, request)
                .await?;
        // build_chat_patch always sets messages, so LiteLLM's missing-follow-up error has no path here
        let messages = patch.messages.unwrap_or_default();
        let mut params = request.optional_params.clone();
        for (key, value) in &patch.optional_params {
            params.insert(key.clone(), value.clone());
        }
        params.shift_remove("tool_choice");
        Ok(ChatRerunRequest {
            model: patch.model.unwrap_or_else(|| request.model.to_owned()),
            messages,
            params,
            kwargs: patch.kwargs,
        })
    }
}

fn websearch_plan(
    request_patch: AgenticLoopRequestPatch,
    response_format: &str,
) -> Result<AgenticLoopPlan, FollowupError> {
    let mut metadata = JsonMap::new();
    metadata.insert("tool_type".to_owned(), to_raw_value("websearch")?);
    metadata.insert("response_format".to_owned(), to_raw_value(response_format)?);
    Ok(AgenticLoopPlan {
        run_agentic_loop: true,
        request_patch: Some(request_patch),
        response_override: None,
        terminate: false,
        stop_reason: None,
        metadata,
    })
}
