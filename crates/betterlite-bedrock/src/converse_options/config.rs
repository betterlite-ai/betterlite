use crate::model_routes::{ParamMap, ParamValue};

pub const CUSTOM_LLM_PROVIDER: &str = "bedrock_converse";

const BLOCKS: [(&str, &str); 3] = [
    ("guardrailConfig", "GuardrailConfigBlock"),
    ("performanceConfig", "PerformanceConfigBlock"),
    ("serviceTier", "ServiceTierBlock"),
];
const CONFIG_FIELDS: [&str; 5] = ["maxTokens", "stopSequences", "temperature", "topP", "topK"];
const CUSTOM_LLM_PROVIDER_FIELD: &str = "custom_llm_provider";
const DESCRIPTOR_KIND_FIELD: &str = "kind";
const DESCRIPTOR_PROPERTY: &str = "property";
const EXCLUDED_DESCRIPTOR_KINDS: [&str; 4] = [
    "builtin_function",
    "classmethod",
    "function",
    "staticmethod",
];

// LiteLLM's __init__ stores every non-None argument on the class; the bare annotations store nothing.
#[must_use]
pub fn class_attributes(arguments: &ParamMap) -> ParamMap {
    let mut attributes = ParamMap::from([(
        CUSTOM_LLM_PROVIDER_FIELD.to_owned(),
        ParamValue::Mapping(ParamMap::from([(
            DESCRIPTOR_KIND_FIELD.to_owned(),
            ParamValue::Text(DESCRIPTOR_PROPERTY.to_owned()),
        )])),
    )]);
    for name in CONFIG_FIELDS {
        let Some(value) = arguments.get(name) else {
            continue;
        };
        if matches!(value, ParamValue::Null) {
            continue;
        }
        attributes.insert(name.to_owned(), value.clone());
    }
    attributes
}

// get_config keeps the property descriptor that BaseConfig drops, so a caller carries its kind instead.
#[must_use]
pub fn get_config(attributes: &ParamMap) -> ParamMap {
    let mut config = ParamMap::new();
    for (name, value) in attributes {
        if name.starts_with('_') || matches!(value, ParamValue::Null) {
            continue;
        }
        if let ParamValue::Mapping(fields) = value
            && let Some(ParamValue::Text(kind)) = fields.get(DESCRIPTOR_KIND_FIELD)
            && EXCLUDED_DESCRIPTOR_KINDS.contains(&kind.as_str())
        {
            continue;
        }
        config.insert(name.clone(), value.clone());
    }
    config
}

#[must_use]
pub fn config_blocks() -> ParamMap {
    BLOCKS
        .iter()
        .map(|(field, block)| ((*field).to_owned(), ParamValue::Text((*block).to_owned())))
        .collect()
}
