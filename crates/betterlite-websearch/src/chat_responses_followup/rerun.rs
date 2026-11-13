use serde::{Deserialize, Serialize};

use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};

use super::ChatResponsesFollowup;
use super::chat::{FollowupRequest, build_chat_patch};
use super::error::FollowupError;
use crate::call_transformation::wire::Call;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatRerunRequest {
    pub model: String,
    pub messages: Vec<FollowupMessage>,
    pub params: JsonMap,
    pub kwargs: JsonMap,
}

impl ChatResponsesFollowup {
    pub async fn build_chat_rerun(
        &self,
        tool_calls: &[Call],
        response_format: &str,
        request: &FollowupRequest<'_>,
    ) -> Result<ChatRerunRequest, FollowupError> {
        let patch =
            build_chat_patch(self.search.as_ref(), tool_calls, response_format, request).await?;
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
