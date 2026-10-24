use std::fmt::Display;

use betterlite_models::catalog_generalizations::{
    CatalogGeneralizations, decode_model_generalization_rules,
};
use betterlite_models::error::ModelsError;
use serde_json::{Value, json};

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogGeneralizationsInput {
    pub(crate) rules: Value,
    pub(crate) routing: Vec<String>,
    pub(crate) capabilities: Vec<String>,
    pub(crate) fill_missing: Vec<FillMissingProbe>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FillMissingProbe {
    pub(crate) model: String,
    pub(crate) provider: String,
}

pub(crate) fn invoke(
    input: &CatalogGeneralizationsInput,
) -> Result<Value, Box<super::HelperError>> {
    let bytes = serde_json::to_vec(&input.rules).map_err(|error| Box::new(value_error(&error)))?;
    let rules =
        decode_model_generalization_rules(&bytes).map_err(|error| Box::new(value_error(&error)))?;
    let (generalizations, warnings) = CatalogGeneralizations::install(rules);
    let fill_missing = input
        .fill_missing
        .iter()
        .map(|probe| generalizations.match_fill_missing(&probe.model, &probe.provider));
    Ok(json!({
        "rules": generalizations.rules,
        "warnings": warnings.iter().map(warning).collect::<Vec<_>>(),
        "routing": input.routing.iter().map(|model| generalizations.match_routing(model)).collect::<Vec<_>>(),
        "capabilities": input.capabilities.iter().map(|model| generalizations.match_capabilities(model)).collect::<Vec<_>>(),
        "fill_missing": fill_missing.collect::<Vec<_>>(),
    }))
}

fn warning(skipped: &ModelsError) -> Value {
    let subject = match skipped {
        ModelsError::MalformedRule { label }
        | ModelsError::MalformedFillMissing { label }
        | ModelsError::InvalidProvider { label } => label.as_str(),
        ModelsError::InvalidRegex { pattern, .. } => pattern.as_str(),
        ModelsError::RulesTooLarge { .. } | ModelsError::RulesJson { .. } => "",
    };
    json!({"subject": subject, "message": skipped.to_string()})
}

fn value_error<E>(error: &E) -> super::HelperError
where
    E: Display,
{
    super::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    }
}

mod tests;
