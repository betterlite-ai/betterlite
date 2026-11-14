use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::message_replay::advisor::strip_advisor_blocks;
use betterlite_anthropic::message_replay::{BlockStrip, strip_blocks};
use betterlite_anthropic::option_primitives::context_management::map_context_management;
use betterlite_models::decisions::Json as ContractJson;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;

use super::adaptive::{AdaptiveEffort, translate_adaptive_effort};
use super::capabilities::{LegacyShapes, ThinkingCapabilities};
use super::error::TransformError;
use super::json::{from_contract, to_contract};
use super::reasoning::{ReasoningAlias, translate_reasoning_effort};
use super::temperature::{TemperatureRule, drop_incompatible_temperature};
use crate::api::messages_native_prepare::billing::filter_billing_headers;

const ADVISOR_TOOL_TYPE: &str = "advisor_20260301";

pub struct TransformRequest<'request> {
    pub model: &'request str,
    pub messages: Vec<Json>,
    pub optional_params: Object,
    pub custom_llm_provider: &'request str,
    pub strips_billing_metadata: bool,
}

pub async fn transform_native_request<Capabilities>(
    capabilities: &mut Capabilities,
    request: TransformRequest<'_>,
) -> Result<Object, TransformError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    let TransformRequest {
        model,
        messages,
        mut optional_params,
        custom_llm_provider,
        strips_billing_metadata,
    } = request;
    let max_tokens = take_max_tokens(&mut optional_params)?;
    translate_reasoning_effort(
        capabilities,
        ReasoningAlias {
            model,
            custom_llm_provider,
            max_tokens,
            optional_params: &mut optional_params,
        },
    )
    .await?;
    apply_thinking_rules(
        capabilities,
        model,
        custom_llm_provider,
        max_tokens,
        &mut optional_params,
    )
    .await?;
    if strips_billing_metadata {
        strip_billing_system(&mut optional_params);
    }
    convert_context_management(&mut optional_params)?;
    Ok(finish_request(model, messages, optional_params, max_tokens))
}

async fn apply_thinking_rules<Capabilities>(
    capabilities: &mut Capabilities,
    model: &str,
    custom_llm_provider: &str,
    max_tokens: i64,
    optional_params: &mut Object,
) -> Result<(), TransformError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    let mut contract = OptionalParams {
        reasoning_effort: None,
        thinking: optional_params.get("thinking").map(to_contract),
        output_config: optional_params.get("output_config").map(to_contract),
    };
    normalize_legacy_shapes(capabilities, model, custom_llm_provider, &mut contract).await?;
    write_contract(optional_params, "thinking", contract.thinking.as_ref())?;
    write_contract(
        optional_params,
        "output_config",
        contract.output_config.as_ref(),
    )?;
    translate_adaptive_effort(
        capabilities,
        AdaptiveEffort {
            model,
            custom_llm_provider,
            max_tokens,
            optional_params,
        },
    )
    .await?;
    drop_incompatible_temperature(
        capabilities,
        TemperatureRule {
            model,
            custom_llm_provider,
            optional_params,
        },
    )
    .await;
    Ok(())
}

async fn normalize_legacy_shapes<Capabilities>(
    capabilities: &mut Capabilities,
    model: &str,
    custom_llm_provider: &str,
    contract: &mut OptionalParams,
) -> Result<(), TransformError>
where
    Capabilities: ThinkingCapabilities + Send,
{
    capabilities
        .drop_disabled_thinking(model, contract, custom_llm_provider)
        .await;
    let mut shapes = LegacyShapes {
        model,
        custom_llm_provider,
        params: contract,
    };
    Ok(capabilities.translate_legacy_thinking(&mut shapes).await?)
}

fn finish_request(
    model: &str,
    messages: Vec<Json>,
    optional_params: Object,
    max_tokens: i64,
) -> Object {
    log_messages(&messages);
    let mut kept = messages;
    if !names_advisor_tool(&optional_params) {
        strip_advisor_blocks(&mut kept, false);
    }
    let stripped = strip_blocks(&kept, BlockStrip::EncryptedReasoning);
    let mut body = Object::new();
    body.insert("messages".to_owned(), Json::List(stripped));
    body.insert(
        "max_tokens".to_owned(),
        Json::Number(Number::from(max_tokens)),
    );
    body.insert("model".to_owned(), Json::Text(model.to_owned()));
    body.extend(optional_params);
    body
}

fn take_max_tokens(params: &mut Object) -> Result<i64, TransformError> {
    match params.shift_remove("max_tokens") {
        None | Some(Json::Null) => Err(TransformError::MissingMaxTokens),
        Some(Json::Number(number)) => number.as_i64().ok_or(TransformError::IntegerField {
            field: "max_tokens",
        }),
        Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
            Err(TransformError::IntegerField {
                field: "max_tokens",
            })
        }
    }
}

fn write_contract(
    params: &mut Object,
    key: &str,
    value: Option<&ContractJson>,
) -> Result<(), TransformError> {
    match value {
        Some(updated) => {
            params.insert(key.to_owned(), from_contract(updated)?);
        }
        None => {
            params.shift_remove(key);
        }
    }
    Ok(())
}

// LiteLLM reads a null system value as absent and leaves it in the request untouched.
fn strip_billing_system(params: &mut Object) {
    let Some(system) = params.get("system") else {
        return;
    };
    if matches!(system, Json::Null) {
        return;
    }
    match filter_billing_headers(system).filter(is_non_empty) {
        Some(filtered) => {
            params.insert("system".to_owned(), filtered);
        }
        None => {
            params.shift_remove("system");
        }
    }
}

// Python's len() on the filtered value: an empty string or mapping counts as removed.
fn is_non_empty(system: &Json) -> bool {
    match system {
        Json::Null => false,
        Json::Bool(_) | Json::Number(_) => true,
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}

fn convert_context_management(params: &mut Object) -> Result<(), TransformError> {
    let Some(context_management) = params.get("context_management") else {
        return Ok(());
    };
    if matches!(context_management, Json::Null) {
        return Ok(());
    }
    let Some(converted) = map_context_management(&to_contract(context_management))? else {
        return Ok(());
    };
    params.insert(
        "context_management".to_owned(),
        from_contract(&ContractJson::Object(converted))?,
    );
    Ok(())
}

fn log_messages(messages: &[Json]) {
    let Ok(payload) = serde_json::to_string(messages) else {
        return;
    };
    tracing::debug!("TRANSFORMATION DEBUG - Messages: {payload}");
}

fn names_advisor_tool(params: &Object) -> bool {
    let Some(Json::List(tools)) = params.get("tools") else {
        return false;
    };
    tools.iter().any(|tool| {
        tool.as_object().is_some_and(|fields| {
            fields.get("type").and_then(Json::as_str) == Some(ADVISOR_TOOL_TYPE)
        })
    })
}
