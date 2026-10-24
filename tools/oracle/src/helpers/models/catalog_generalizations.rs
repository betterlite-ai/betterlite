use betterlite_models::catalog_generalizations::{
    CatalogGeneralizations, decode_generalization_rules,
};
use betterlite_models::error::ModelsError;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
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

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let rules =
            decode_generalization_rules(input.rules.to_string().as_bytes()).map_err(|error| {
                Box::new(HelperError {
                    class: "ValueError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        Ok(report(input, rules))
    }
}

fn report(input: &Input, rules: Vec<Box<serde_json::value::RawValue>>) -> Value {
    let (installed, warnings) = CatalogGeneralizations::install(rules);
    let fill_missing = input
        .fill_missing
        .iter()
        .map(|probe| installed.match_fill_missing(&probe.model, &probe.provider));
    json!({
        "rules": installed.rules,
        "warnings": warnings.iter().map(warning).collect::<Vec<_>>(),
        "routing": input.routing.iter().map(|model| installed.match_routing(model)).collect::<Vec<_>>(),
        "capabilities": input.capabilities.iter().map(|model| installed.match_capabilities(model)).collect::<Vec<_>>(),
        "fill_missing": fill_missing.collect::<Vec<_>>(),
    })
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

mod tests;
