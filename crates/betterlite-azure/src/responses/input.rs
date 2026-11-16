use betterlite_openai::response_input::{ResponsesInput, ResponsesItem};
use betterlite_prompts::arguments::json::Json;

use super::reasoning::sanitize_item;

// Azure validates the reasoning items itself, then removes 'status' from every message item
pub(super) fn azure_input(input: &ResponsesInput) -> ResponsesInput {
    let ResponsesInput::Items(items) = input else {
        return input.clone();
    };
    ResponsesInput::Items(
        items
            .iter()
            .map(|item| ResponsesItem(prepared_item(&item.0)))
            .collect(),
    )
}

fn prepared_item(item: &Json) -> Json {
    let Json::Object(item_fields) = item else {
        return item.clone();
    };
    let is_reasoning =
        matches!(item_fields.get("type"), Some(Json::Text(kind)) if kind == "reasoning");
    let is_message = matches!(item_fields.get("type"), Some(Json::Text(kind)) if kind == "message");
    if is_reasoning {
        return Json::Object(sanitize_item(item_fields));
    }
    let mut fields = item_fields.clone();
    if is_message {
        fields.shift_remove("status");
    }
    Json::Object(fields)
}
