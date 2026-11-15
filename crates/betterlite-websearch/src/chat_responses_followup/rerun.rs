use serde::{Deserialize, Serialize};

use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatRerunRequest {
    pub model: String,
    pub messages: Vec<FollowupMessage>,
    pub params: JsonMap,
    pub kwargs: JsonMap,
}
