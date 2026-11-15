use crate::model_routes::{ParamMap, ParamValue};

pub const CUSTOM_LLM_PROVIDER: &str = "bedrock_converse";

const BLOCKS: [(&str, &str); 3] = [
    ("guardrailConfig", "GuardrailConfigBlock"),
    ("performanceConfig", "PerformanceConfigBlock"),
    ("serviceTier", "ServiceTierBlock"),
];

// LiteLLM's __init__ stores every non-None argument on the class; the bare annotations store nothing.
#[must_use]
pub fn class_attributes(arguments: &ParamMap) -> ParamMap {
    let mut attributes = ParamMap::from([(
        "custom_llm_provider".to_owned(),
        ParamValue::Mapping(ParamMap::from([(
            "kind".to_owned(),
            ParamValue::Text("property".to_owned()),
        )])),
    )]);
    for name in ["maxTokens", "stopSequences", "temperature", "topP", "topK"] {
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
            && let Some(ParamValue::Text(kind)) = fields.get("kind")
            && [
                "builtin_function",
                "classmethod",
                "function",
                "staticmethod",
            ]
            .contains(&kind.as_str())
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
