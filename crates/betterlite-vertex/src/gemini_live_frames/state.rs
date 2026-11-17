use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CallerState {
    pub output_item_id: Option<String>,
    pub response_id: Option<String>,
    pub conversation_id: Option<String>,
    pub delta_chunks: Option<Vec<Json>>,
    pub item_chunks: Option<Vec<Json>>,
    pub delta_type: Option<AllDeltaType>,
    pub session_configuration_request: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameResponse {
    pub events: Vec<Json>,
    pub state: CallerState,
}
