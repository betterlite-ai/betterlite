use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;
use indexmap::IndexMap;

use crate::anthropic_capability_catalog::CatalogSources;
use crate::capabilities::effort::validate_effort_for_model;
use crate::capabilities::effort_param::model_supports_effort_param;
use crate::params::error::ParamsError;

pub const DROP_OUTPUT_CONFIG_WARNING: &str = "Dropping unsupported `output_config` for model=%s (drop_params=True). Effort is only supported on Opus 4.5+, Sonnet 4.6+, and Mythos Preview.";

const VALID_EFFORTS: [&str; 5] = ["high", "medium", "low", "xhigh", "max"];

pub struct OutputConfigRequest<'request> {
    pub model: &'request str,
    pub provider: &'request str,
    pub global_drop: bool,
}

// LiteLLM's repr() in the message, close enough that the offending value reads as Python wrote it
fn rendered(value: &Json) -> String {
    match value {
        Json::Text(text) => format!("'{text}'"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

fn preserve_format(
    config: &IndexMap<String, Json>,
    data: &mut IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
) {
    let preserved = config
        .get("format")
        .filter(|format| !matches!(format, Json::Null));
    match preserved {
        Some(format) => {
            let mut format_only = IndexMap::new();
            format_only.insert("format".to_owned(), format.clone());
            optional_params.insert(
                "output_config".to_owned(),
                Json::Object(format_only.clone()),
            );
            data.insert("output_config".to_owned(), Json::Object(format_only));
        }
        None => {
            optional_params.shift_remove("output_config");
            data.shift_remove("output_config");
        }
    }
}

pub async fn apply<Source, Transport>(
    request: &OutputConfigRequest<'_>,
    data: &mut IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
    environment: &mut CapabilityEnvironment<'_, Source, Transport>,
    catalogs: &CatalogSources<'_>,
) -> Result<(), ParamsError>
where
    Source: DynamicMetadataSource,
    Transport: ModelInfoClient,
{
    let Some(Json::Object(config)) = optional_params.get("output_config") else {
        return Ok(());
    };
    if config.is_empty() {
        return Ok(());
    }
    if request.global_drop
        && config.keys().any(|key| key != "format")
        && !model_supports_effort_param(request.model, request.provider, environment, catalogs)
            .await
    {
        tracing::warn!(
            "{}",
            DROP_OUTPUT_CONFIG_WARNING.replacen("%s", request.model, 1)
        );
        preserve_format(config, data, optional_params);
        return Ok(());
    }
    let validated = match config.get("effort") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) if VALID_EFFORTS.contains(&text.as_str()) => Some(text.as_str()),
        Some(other) => {
            return Err(ParamsError::InvalidEffort {
                value: rendered(other),
            });
        }
    };
    if let Some(message) = validate_effort_for_model(
        request.model,
        validated,
        request.provider,
        environment,
        catalogs,
    )
    .await
    {
        return Err(ParamsError::EffortUnsupported { message });
    }
    data.insert("output_config".to_owned(), Json::Object(config.clone()));
    Ok(())
}
