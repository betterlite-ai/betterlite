use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::content_filter_guard::ContentFilterGuardrail;
use betterlite_guardrails::content_filter_inputs::images::ImageDescriptionCompletion;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract as _;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use serde_json::Value;
use serde_json::json;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::HelperResult;

use super::ApplyInput;
use super::failure;

// The fields a caller reads back from the constructed class, as LiteLLM stores them
pub(super) fn build<R: ImageDescriptionCompletion>(
    guardrail: &ContentFilterGuardrail<R>,
) -> HelperResult {
    let patterns = guardrail
        .compiled_patterns
        .iter()
        .map(|pattern| json!({"name": pattern.pattern_name, "action": pattern.action}))
        .collect::<Vec<_>>();
    let mut blocked_words = serde_json::Map::new();
    for (keyword, (action, description)) in &guardrail.blocked_words {
        blocked_words.insert(keyword.clone(), json!([action, description]));
    }
    let event_hook = serde_json::to_value(&guardrail.base.settings.event_hook)
        .map_err(|error| failure("JsonError", error.to_string()))?;
    Ok(json!({
        "guardrail_provider": guardrail.guardrail_provider,
        "config_guardrail_id": guardrail.config_guardrail_id,
        "config_policy_template": guardrail.config_policy_template,
        "pattern_redaction_format": guardrail.pattern_redaction_format,
        "keyword_redaction_tag": guardrail.keyword_redaction_tag,
        "severity_threshold": guardrail.severity_threshold,
        "event_hook": event_hook,
        "default_on": guardrail.base.settings.default_on,
        "image_model": guardrail.image_model,
        "patterns": patterns,
        "blocked_words": blocked_words,
        "supported_event_hooks": supported_event_hooks(guardrail)?,
        "config_model": config_model(guardrail)?,
    }))
}

// The run's visible effect: the rewritten inputs, or the error LiteLLM raised
pub(super) async fn apply<R: ImageDescriptionCompletion>(
    guardrail: &ContentFilterGuardrail<R>,
    input: &ApplyInput,
) -> HelperResult {
    let mut request_data: BTreeMap<String, GuardrailValue> = BTreeMap::new();
    let inputs = GuardrailApiInputs {
        texts: Some(input.texts.clone()),
        images: None,
        tools: None,
        tool_calls: input.tool_calls.clone(),
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    match guardrail
        .apply_guardrail(inputs, &mut request_data, input.input_type, None)
        .await
    {
        Ok(returned) => {
            let tool_calls = match returned.tool_calls {
                Some(calls) => json!(tool_tags(&calls)),
                None => Value::Null,
            };
            Ok(json!({
                "texts": returned.texts,
                "tool_calls": tool_calls,
                "error": Value::Null,
            }))
        }
        Err(failed) => Ok(json!({
            "texts": Value::Null,
            "tool_calls": Value::Null,
            "error": error_tag(&failed),
        })),
    }
}

fn supported_event_hooks<R: ImageDescriptionCompletion>(
    guardrail: &ContentFilterGuardrail<R>,
) -> Result<Value, Box<HelperError>> {
    let Some(hooks) = guardrail.get_supported_event_hooks() else {
        return Ok(Value::Null);
    };
    serde_json::to_value(hooks).map_err(|error| failure("JsonError", error.to_string()))
}

// LiteLLM's get_config_model answers with a class, whose name the config endpoint reads
fn config_model<R: ImageDescriptionCompletion>(
    guardrail: &ContentFilterGuardrail<R>,
) -> Result<Value, Box<HelperError>> {
    let Some(model) = guardrail.get_config_model() else {
        return Ok(Value::Null);
    };
    let value =
        serde_json::to_value(model).map_err(|error| failure("JsonError", error.to_string()))?;
    Ok(value.get("model").cloned().unwrap_or(Value::Null))
}

fn error_tag(outcome: &GuardrailFailure) -> Value {
    match outcome {
        GuardrailFailure::Http { status, detail } => {
            json!({"kind": "http", "status": status, "detail": detail})
        }
        GuardrailFailure::HttpObject { status, detail } => {
            json!({"kind": "http", "status": status, "detail": json!(detail)})
        }
        GuardrailFailure::ModifyResponse(passthrough) => {
            json!({"kind": "modify_response", "message": passthrough.message})
        }
        failure @ (GuardrailFailure::GuardrailRaised { .. }
        | GuardrailFailure::BlockedPii { .. }
        | GuardrailFailure::SensitiveDataRoute(_)
        | GuardrailFailure::Other(_)) => {
            json!({"kind": "failure", "message": failure.to_string()})
        }
    }
}

fn tool_tags(calls: &[GuardrailValue]) -> Vec<Value> {
    let mut tags = Vec::with_capacity(calls.len());
    for call in calls {
        if let Some(tag) = tool_tag(call) {
            tags.push(tag);
        }
    }
    tags
}

fn tool_tag(call: &GuardrailValue) -> Option<Value> {
    let GuardrailValue::Object(fields) = call else {
        return None;
    };
    let GuardrailValue::Object(function) = fields.get("function")? else {
        return None;
    };
    let GuardrailValue::Text(name) = function.get("name")? else {
        return None;
    };
    let GuardrailValue::Text(arguments) = function.get("arguments")? else {
        return None;
    };
    // LiteLLM rewrites the argument text in place, so the case compares its JSON, not its spacing
    let parsed = serde_json::from_str(arguments)
        .unwrap_or_else(|_invalid| Value::String(arguments.to_owned()));
    Some(json!({"name": name, "arguments": parsed}))
}
