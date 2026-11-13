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
use self::responses::ResponsesRequest;
use self::search::SearchExecutor;
use crate::call_transformation::wire::Call;

pub struct ChatResponsesFollowup {
    pub search: Arc<dyn SearchExecutor>,
}

impl ChatResponsesFollowup {
    pub async fn build_chat_plan(
        &self,
        tool_calls: &[Call],
        response_format: &str,
        request: &FollowupRequest<'_>,
    ) -> Result<AgenticLoopPlan, FollowupError> {
        let request_patch =
            chat::build_chat_patch(self.search.as_ref(), tool_calls, response_format, request)
                .await?;
        websearch_plan(request_patch, response_format)
    }

    pub async fn build_responses_plan(
        &self,
        tool_calls: &[Call],
        request: &ResponsesRequest<'_>,
    ) -> Result<AgenticLoopPlan, FollowupError> {
        let request_patch =
            responses::build_responses_patch(self.search.as_ref(), tool_calls, request).await?;
        websearch_plan(request_patch, "responses")
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
