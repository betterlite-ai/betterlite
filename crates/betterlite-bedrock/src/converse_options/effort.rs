use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::capabilities::effort::validate_effort_for_model;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::converse_options::error::{
    BEDROCK_CONVERSE_PROVIDER, ConverseOptionsError, NOVA_2_REJECTION_MODEL,
};
use crate::model_routes::{ParamMap, ParamValue};

const ADAPTIVE_EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
const NOVA_2_EFFORTS: [&str; 3] = ["low", "medium", "high"];

pub fn transform_reasoning_effort_to_reasoning_config(
    reasoning_effort: &str,
) -> Result<ParamMap, ConverseOptionsError> {
    if !NOVA_2_EFFORTS.contains(&reasoning_effort) {
        return Err(ConverseOptionsError::NovaEffort {
            value: reasoning_effort.to_owned(),
            model: NOVA_2_REJECTION_MODEL.to_owned(),
            provider: BEDROCK_CONVERSE_PROVIDER.to_owned(),
        });
    }
    let mut config = ParamMap::new();
    config.insert("type".to_owned(), ParamValue::Text("enabled".to_owned()));
    config.insert(
        "maxReasoningEffort".to_owned(),
        ParamValue::Text(reasoning_effort.to_owned()),
    );
    Ok(ParamMap::from([(
        "reasoningConfig".to_owned(),
        ParamValue::Mapping(config),
    )]))
}

// LiteLLM's REASONING_EFFORT_TO_OUTPUT_CONFIG_EFFORT: minimal folds into low, every other level stays.
#[must_use]
pub fn output_config_effort(effort: &str) -> Option<&'static str> {
    match effort {
        "low" | "minimal" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        _ => None,
    }
}

// The membership check runs first; only a permitted effort reaches the model-capability check.
pub async fn validate_anthropic_adaptive_effort<Source, Transport>(
    model: &str,
    effort: &ParamValue,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Result<(), ConverseOptionsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let ParamValue::Text(text) = effort else {
        return Err(ConverseOptionsError::AdaptiveEffort {
            label: effort_label(effort),
            model: model.to_owned(),
            provider: BEDROCK_CONVERSE_PROVIDER.to_owned(),
        });
    };
    if !ADAPTIVE_EFFORTS.contains(&text.as_str()) {
        return Err(ConverseOptionsError::AdaptiveEffort {
            label: effort_label(effort),
            model: model.to_owned(),
            provider: BEDROCK_CONVERSE_PROVIDER.to_owned(),
        });
    }
    match validate_effort_for_model(model, Some(text), "bedrock", environment, catalogs).await {
        Some(message) => Err(ConverseOptionsError::UnsupportedEffort {
            message,
            model: model.to_owned(),
            provider: BEDROCK_CONVERSE_PROVIDER.to_owned(),
        }),
        None => Ok(()),
    }
}

// LiteLLM's f-string quotes a string with single quotes; the other JSON shapes keep their Python repr.
#[must_use]
pub fn effort_label(effort: &ParamValue) -> String {
    match effort {
        ParamValue::Text(text) => format!("'{text}'"),
        ParamValue::Bool(true) => "True".to_owned(),
        ParamValue::Bool(false) => "False".to_owned(),
        ParamValue::Null => "None".to_owned(),
        ParamValue::Number(number) => number.to_string(),
        ParamValue::List(items) => {
            let rendered: Vec<String> = items.iter().map(effort_label).collect();
            format!("[{}]", rendered.join(", "))
        }
        ParamValue::Mapping(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|(key, value)| format!("'{key}': {}", effort_label(value)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}
