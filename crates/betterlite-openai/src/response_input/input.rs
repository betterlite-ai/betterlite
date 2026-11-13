use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json, Object};

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

fn validated_item(item: &Object) -> Object {
    match item.get("type") {
        Some(Json::Text(kind)) if kind == "reasoning" => handle_reasoning_item(item),
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
    let prefix = match item.0.get("type") {
        Some(Json::Text(kind)) if kind == "function_call" => "fc",
        Some(Json::Text(kind)) if kind == "custom_tool_call" => "ctc",
        _ => return item,
    };
    let Some(Json::Text(id)) = item.0.get("id") else {
        return item;
    };
    if id.starts_with(prefix) {
        return item;
    }
    let mut fields = item.0;
    fields.shift_remove("id");
    ResponsesItem(fields)
}
