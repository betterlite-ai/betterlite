use betterlite_config::sections::bedrock_agentcore_request::ForwardingValue;
use indexmap::IndexMap;

use super::message::AgentCoreContent;
use crate::model_routes::{ParamMap, ParamValue};

pub(super) fn forward_multimodal_content(
    optional_params: &ParamMap,
    litellm_params: &ParamMap,
) -> bool {
    for source in [optional_params, litellm_params] {
        let Some(value) = source.get("forward_multimodal_content") else {
            continue;
        };
        // Python skips a null value and lets the next mapping decide
        if matches!(value, ParamValue::Null) {
            continue;
        }
        return forwarding_value(value).is_enabled();
    }
    false
}

// any block whose type is neither absent nor "text" opts the whole list in, text blocks included
pub(super) fn has_non_text_block(content: &AgentCoreContent) -> bool {
    match content {
        AgentCoreContent::Text(_) => false,
        AgentCoreContent::Parts(parts) => parts
            .iter()
            .any(|part| part.kind.as_deref().is_some_and(|kind| kind != "text")),
    }
}

fn forwarding_value(value: &ParamValue) -> ForwardingValue {
    match value {
        ParamValue::Null => ForwardingValue::Null,
        ParamValue::Bool(flag) => ForwardingValue::Boolean(*flag),
        ParamValue::Number(number) => number_value(number),
        ParamValue::Text(text) => ForwardingValue::Text(text.clone()),
        ParamValue::List(items) => {
            ForwardingValue::List(items.iter().map(forwarding_value).collect())
        }
        ParamValue::Mapping(entries) => ForwardingValue::Mapping(
            entries
                .iter()
                .map(|(name, entry)| (name.clone(), forwarding_value(entry)))
                .collect::<IndexMap<_, _>>(),
        ),
    }
}

fn number_value(number: &serde_json::Number) -> ForwardingValue {
    if let Some(value) = number.as_i64() {
        return ForwardingValue::Integer(value);
    }
    if let Some(value) = number.as_u64() {
        return ForwardingValue::Unsigned(value);
    }
    ForwardingValue::Float(number.as_f64().unwrap_or(f64::NAN))
}
