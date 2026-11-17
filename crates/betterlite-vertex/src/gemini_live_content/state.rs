use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentState {
    pub output_item_id: Option<String>,
    pub response_id: Option<String>,
    pub conversation_id: Option<String>,
    pub delta_chunks: Option<Vec<Json>>,
    pub item_chunks: Option<Vec<Json>>,
    pub delta_type: Option<AllDeltaType>,
}
