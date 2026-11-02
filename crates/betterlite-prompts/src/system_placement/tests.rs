#![cfg(test)]

use super::*;
use crate::arguments::json::{Json, Object};
use crate::message_content::{Content, ContentPart};

fn message(role: &str, content: Option<Content>) -> Message {
    Message {
        role: role.to_owned(),
        content,
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn text_message(role: &str, text: &str) -> Message {
    message(role, Some(Content::Text(text.to_owned())))
}

fn parts_message(role: &str, parts: Vec<ContentPart>) -> Message {
    message(role, Some(Content::Parts(parts)))
}

fn text_part(text: &str) -> ContentPart {
    ContentPart {
        kind: "text".to_owned(),
        fields: Object::from_iter([("text".to_owned(), Json::Text(text.to_owned()))]),
    }
}

fn thinking_part(thinking: &str, signature: Option<&str>) -> ContentPart {
    let mut fields = Object::from_iter([("thinking".to_owned(), Json::Text(thinking.to_owned()))]);
    if let Some(value) = signature {
        fields.insert("signature".to_owned(), Json::Text(value.to_owned()));
    }
    ContentPart {
        kind: "thinking".to_owned(),
        fields,
    }
}

fn ephemeral_cache_control(ttl: Option<&str>) -> Json {
    let mut fields = Object::from_iter([("type".to_owned(), Json::Text("ephemeral".to_owned()))]);
    if let Some(value) = ttl {
        fields.insert("ttl".to_owned(), Json::Text(value.to_owned()));
    }
    Json::Object(fields)
}

fn media_part(kind: &str) -> ContentPart {
    ContentPart {
        kind: kind.to_owned(),
        fields: Object::new(),
    }
}

mod an_all_system_message_list_still_runs_the_conversion_policy;
mod anthropic_system_messages_omits_text_less_runs;
mod cache_control_normalizes_known_and_unknown_ttls;
mod flagged_anchor_renders_through_image_document_and_file_parts;
mod flagged_anchor_renders_through_server_tool_use_and_tool_result_parts;
mod flagged_anchor_requires_a_rendering_neighbour;
mod flagged_run_after_assistant_turn_slides_to_the_next_user_turn;
mod flagged_run_after_user_turn_stays_with_that_turn;
mod flagged_run_between_assistant_turns_converts_in_place;
mod flagged_run_ending_the_array_converts_in_place;
mod flagged_runs_anchored_to_the_same_turn_merge;
mod inline_thinking_part_suppresses_separate_thinking_blocks;
mod leading_system_run_precondition_violation_is_handled_safely;
mod no_system_message_is_left_untouched;
mod split_leading_system_run_separates_the_leading_run_only;
mod string_content_cache_control_survives_conversion;
mod system_message_as_user_always_includes_the_operator_note;
mod thinking_blocks_decide_whether_a_message_renders;
mod unflagged_model_converts_every_system_message_to_a_user_turn;
mod unflagged_run_before_a_tool_result_moves_after_it;
