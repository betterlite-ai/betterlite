use betterlite_prompts::arguments::json::Json;

use crate::api::summary_history::boundary::block_has_type;
use crate::api::summary_history::system::text_turn;

pub const CONTINUE_PROMPT: &str = "Please continue based on the conversation summary above.";

// a tool_result loses its tool_use turn after compaction and would become an orphaned tool message
#[must_use]
pub fn select_last_user_question(messages: &[Json]) -> Json {
    for message in messages.iter().rev() {
        let Json::Object(fields) = message else {
            continue;
        };
        if fields.get("role").and_then(Json::as_str) != Some("user") {
            continue;
        }
        let Some(Json::List(blocks)) = fields.get("content") else {
            return message.clone();
        };
        let kept: Vec<Json> = blocks
            .iter()
            .filter(|block| !block_has_type(block, "tool_result"))
            .cloned()
            .collect();
        if !kept.is_empty() {
            let mut question = fields.clone();
            question.insert("content".to_owned(), Json::List(kept));
            return Json::Object(question);
        }
    }
    text_turn("user", CONTINUE_PROMPT)
}
