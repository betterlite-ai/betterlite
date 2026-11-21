use betterlite_prompts::arguments::json::Json;

use crate::api::summary_history::boundary::{blocks_without, listed_content, with_content};
use crate::api::summary_history::system::text_turn;

pub const CONTINUE_PROMPT: &str = "Please continue based on the conversation summary above.";

// a tool_result loses its tool_use turn after compaction and would become an orphaned tool message
#[must_use]
pub fn select_last_user_question(messages: &[Json]) -> Json {
    for message in messages.iter().rev() {
        let Some(fields) = message.as_object() else {
            continue;
        };
        if fields.get("role").and_then(Json::as_str) != Some("user") {
            continue;
        }
        let Some((_, blocks)) = listed_content(message) else {
            return message.clone();
        };
        let kept = blocks_without(blocks, "tool_result");
        if !kept.is_empty() {
            return with_content(fields, kept);
        }
    }
    text_turn("user", CONTINUE_PROMPT)
}
