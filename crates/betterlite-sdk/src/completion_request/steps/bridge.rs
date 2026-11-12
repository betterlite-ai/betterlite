use super::Preparation;
use super::convert::{json_from_value, json_value, typed_from};
use super::messages::text_of;
use super::params::provider_name;
use crate::completion_request::error::RequestResult;
use betterlite_context::non_default_params::reasoning_aliases::{
    peek_reasoning_summary_aliases, strip_reasoning_aliases,
};
use betterlite_core::responses_bridge::{
    BridgeCheck, BridgeRequest, ReasoningEffort, responses_api_bridge_check,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
const RESPONSES_MODE: &str = "responses";
fn mode<'state>(state: &'state Preparation<'_>) -> Option<&'state str> {
    state.call.model_info.get("mode").and_then(text_of)
}
fn reasoning_effort(state: &Preparation<'_>) -> RequestResult<Option<ReasoningEffort>> {
    match &state.args.reasoning_effort {
        Some(value) => Ok(Some(typed_from::<ReasoningEffort>(
            value,
            "reasoning_effort",
        )?)),
        None => Ok(None),
    }
}
fn run_check(
    state: &Preparation<'_>,
    tools: Option<&[Json]>,
    effort: Option<&ReasoningEffort>,
    summary: Option<&Json>,
) -> RequestResult<BridgeCheck> {
    let tool_values = tools
        .unwrap_or_default()
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let web_search = match &state.args.web_search_options {
        Some(options) => Some(serde_json::to_value(options)?),
        None => None,
    };
    let summary_value = match summary {
        Some(value) => Some(serde_json::to_value(value)?),
        None => None,
    };
    let request = BridgeRequest {
        model: &state.call.model,
        custom_llm_provider: &provider_name(state),
        web_search_options: web_search.as_ref(),
        tools: &tool_values,
        reasoning_effort: effort,
        reasoning_summary: summary_value.as_ref(),
        api_base: state.call.api_base.as_deref(),
        route_all_openai_to_responses: state.settings.route_all_chat_openai_to_responses,
    };
    Ok(responses_api_bridge_check(&request, state.ports.bridge))
}
fn store(state: &mut Preparation<'_>, check: BridgeCheck) -> RequestResult<()> {
    let value = serde_json::to_value(check.model_info)?;
    state.call.model_info = serde_json::from_value(value)?;
    state.call.model = check.model;
    Ok(())
}
pub(crate) fn early(state: &mut Preparation<'_>) -> RequestResult<()> {
    let check = run_check(state, None, None, None)?;
    store(state, check)
}
fn place_effort(state: &mut Preparation<'_>, effort: JsonValue) {
    state
        .call
        .options
        .params
        .insert(String::from("reasoning_effort"), effort);
}
fn strip_aliases(state: &mut Preparation<'_>) -> RequestResult<Option<Json>> {
    let (stripped, summary) = strip_reasoning_aliases(&state.call.options.params);
    state.call.options.params = stripped;
    summary.as_ref().map(json_from_value).transpose()
}
fn merged_effort(current: Option<JsonValue>, summary: JsonValue) -> JsonValue {
    match current {
        Some(JsonValue::Object(mut fields)) => {
            fields.insert(String::from("summary"), summary);
            JsonValue::Object(fields)
        }
        Some(effort) => {
            let fields = IndexMap::from([
                (String::from("effort"), effort),
                (String::from("summary"), summary),
            ]);
            JsonValue::Object(fields)
        }
        None => JsonValue::Object(IndexMap::from([(String::from("summary"), summary)])),
    }
}
fn bridge_effort(state: &mut Preparation<'_>) -> RequestResult<()> {
    let found = strip_aliases(state)?;
    let object_with_summary = matches!(
        &state.args.reasoning_effort,
        Some(Json::Object(fields)) if fields.contains_key("summary")
    );
    if object_with_summary {
        if let Some(effort) = &state.args.reasoning_effort {
            place_effort(state, json_value(effort)?);
        }
        return Ok(());
    }
    let Some(alias) = found else {
        return Ok(());
    };
    let fallback = match &state.args.reasoning_effort {
        Some(effort) => Some(json_value(effort)?),
        None => None,
    };
    let current = state
        .call
        .options
        .params
        .get("reasoning_effort")
        .cloned()
        .or(fallback);
    place_effort(state, merged_effort(current, json_value(&alias)?));
    Ok(())
}
pub(crate) fn late(state: &mut Preparation<'_>) -> RequestResult<bool> {
    if mode(state) != Some(RESPONSES_MODE) {
        let effort = reasoning_effort(state)?;
        let summary = peek_reasoning_summary_aliases(&state.call.options.params)
            .map(json_from_value)
            .transpose()?;
        let check = run_check(
            state,
            state.call.tools.as_deref(),
            effort.as_ref(),
            summary.as_ref(),
        )?;
        store(state, check)?;
    }
    if mode(state) == Some(RESPONSES_MODE) && !state.skip_bridge {
        bridge_effort(state)?;
        return Ok(true);
    }
    let provider = provider_name(state);
    let azure_model = state
        .call
        .base_model
        .as_deref()
        .unwrap_or(&state.call.model);
    let gpt5_on_chat = (provider == "openai"
        && state.ports.bridge.is_gpt_5_model(&state.call.model))
        || (provider == "azure" && state.ports.bridge.is_gpt_5_model(azure_model));
    if gpt5_on_chat {
        strip_aliases(state)?;
    }
    Ok(false)
}
