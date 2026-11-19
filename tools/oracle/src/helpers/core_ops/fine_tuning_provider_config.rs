use std::collections::BTreeMap;

use betterlite_config::api::fine_tuning_provider_config::{
    get_fine_tuning_provider_config, set_fine_tuning_config,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaseInput {
    env: BTreeMap<String, String>,
    steps: Vec<Step>,
}

// one settings call: a set carries the config, a get names the provider
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    action: Action,
    #[serde(default)]
    config: Option<Json>,
    #[serde(default)]
    provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Set,
    Get,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = CaseInput;

    fn run(input: &Self::Input) -> HelperResult {
        let mut current: Option<Vec<Json>> = None;
        let mut results = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            match step.action {
                Action::Set => {
                    current = set_fine_tuning_config(current, step.config.as_ref(), &input.env)
                        .or_value_error()?;
                    results.push(Json::Null);
                }
                Action::Get => {
                    let Some(provider) = step.provider.as_deref() else {
                        return Err(Box::new(HelperError {
                            class: "ValueError".to_owned(),
                            status: None,
                            error_type: Some(None),
                            param: Some(None),
                            code: Some(None),
                            message: "'provider'".to_owned(),
                        }));
                    };
                    let found = get_fine_tuning_provider_config(current.as_deref(), provider)
                        .or_value_error()?;
                    results.push(found.unwrap_or(Json::Null));
                }
            }
        }
        Ok(json!({ "results": results }))
    }
}
