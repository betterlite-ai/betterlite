use betterlite_models::model_capability_predicates::CapabilityPredicates;
use betterlite_openai::chat::Tool;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use crate::oci_chat::config::{OciParam, param_map, uses_max_completion_tokens};
use crate::oci_chat::payload::format::{normalize_response_format, normalize_tool_choice};
use crate::oci_chat::payload::{encode_error, json_of};
use crate::oci_chat_wire::cohere::request::adapt_cohere_tools;
use crate::oci_chat_wire::generic::request::adapt_generic_tools;
use crate::oci_chat_wire::types::oci::OciVendor;
use crate::oci_common::OciError;

const DEFAULT_MAX_TOKENS: i64 = 4096;

pub(super) async fn get_optional_params<Predicates>(
    vendor: OciVendor,
    optional_params: &Object,
    model: &str,
    predicates: &mut Predicates,
) -> Result<Object, OciError>
where
    Predicates: CapabilityPredicates + Send,
{
    let token_key = choose_token_key(vendor, model, predicates).await;
    let mut selected = select_params(vendor, optional_params, token_key);
    normalize_reasoning_effort(&mut selected);
    if let Some(tools) = selected.get_mut("tools") {
        *tools = adapt_tools(tools, vendor)?;
    }
    if let Some(choice) = selected.get_mut("toolChoice") {
        *choice = normalize_tool_choice(choice)?;
    }
    if let Some(format) = selected.get_mut("responseFormat") {
        *format = normalize_response_format(format, vendor)?;
    }
    Ok(selected)
}

async fn choose_token_key<Predicates>(
    vendor: OciVendor,
    model: &str,
    predicates: &mut Predicates,
) -> &'static str
where
    Predicates: CapabilityPredicates + Send,
{
    if vendor == OciVendor::Cohere
        || model.is_empty()
        || !uses_max_completion_tokens(model, predicates).await
    {
        "maxTokens"
    } else {
        "maxCompletionTokens"
    }
}

fn select_params(vendor: OciVendor, optional_params: &Object, token_key: &str) -> Object {
    let mut selected = Object::new();
    for (openai_key, alias) in param_map(vendor) {
        let OciParam::Oci(oci_alias) = alias else {
            continue;
        };
        let target = if oci_alias == "maxTokens" {
            token_key
        } else {
            oci_alias
        };
        if selected.contains_key(target) {
            continue;
        }
        if let Some(value) = optional_params
            .get(openai_key)
            .or_else(|| optional_params.get(oci_alias))
        {
            selected.insert(target.to_owned(), value.clone());
        }
    }
    selected
        .entry(token_key.to_owned())
        .or_insert(Json::Number(Number::from(DEFAULT_MAX_TOKENS)));
    selected
}

fn normalize_reasoning_effort(selected: &mut Object) {
    let Some(Json::Text(effort)) = selected.get("reasoningEffort") else {
        return;
    };
    let upper = effort.to_uppercase();
    let normalized = if upper == "DISABLE" {
        String::from("NONE")
    } else {
        upper
    };
    selected.insert(String::from("reasoningEffort"), Json::Text(normalized));
}

fn adapt_tools(tools: &Json, vendor: OciVendor) -> Result<Json, OciError> {
    let bytes = serde_json::to_vec(tools).map_err(|source| encode_error(&source))?;
    let parsed: Vec<Tool> = serde_json::from_slice(&bytes).map_err(|source| OciError {
        status: 400,
        message: format!("`tools` is not a list of OpenAI tool definitions: {source}"),
    })?;
    match vendor {
        OciVendor::Cohere => json_of(&adapt_cohere_tools(&parsed)?),
        OciVendor::Generic => json_of(&adapt_generic_tools(&parsed)?),
    }
}
