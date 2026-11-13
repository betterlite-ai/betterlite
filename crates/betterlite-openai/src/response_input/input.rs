use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Json;

use super::reasoning::handle_reasoning_item;
use super::{ResponsesInput, ResponsesItem};

pub(crate) fn validate_input(input: &ResponsesInput) -> ResponsesInput {
    let ResponsesInput::Items(items) = input else {
        return input.clone();
    };
    ResponsesInput::Items(
        items
            .iter()
            .map(|item| ResponsesItem(validated_item(&item.0)))
            .collect(),
    )
}

fn validated_item(item: &Json) -> Json {
    let Json::Object(fields) = item else {
        return item.clone();
    };
    match fields.get("type") {
        Some(Json::Text(kind)) if kind == "reasoning" => {
            Json::Object(handle_reasoning_item(fields))
        }
        _ => item.clone(),
    }
}

pub(crate) fn without_foreign_item_ids(
    provider: ProviderId,
    input: ResponsesInput,
) -> ResponsesInput {
    if !matches!(provider, ProviderId::Openai | ProviderId::Azure) {
        return input;
    }
    let ResponsesInput::Items(items) = input else {
        return input;
    };
    ResponsesInput::Items(items.into_iter().map(without_foreign_item_id).collect())
}

fn without_foreign_item_id(item: ResponsesItem) -> ResponsesItem {
    let Json::Object(mut fields) = item.0 else {
        return item;
    };
    let prefix = match fields.get("type") {
        Some(Json::Text(kind)) if kind == "function_call" => "fc",
        Some(Json::Text(kind)) if kind == "custom_tool_call" => "ctc",
        _ => return ResponsesItem(Json::Object(fields)),
    };
    let Some(Json::Text(id)) = fields.get("id") else {
        return ResponsesItem(Json::Object(fields));
    };
    if id.starts_with(prefix) {
        return ResponsesItem(Json::Object(fields));
    }
    fields.shift_remove("id");
    ResponsesItem(Json::Object(fields))
}
