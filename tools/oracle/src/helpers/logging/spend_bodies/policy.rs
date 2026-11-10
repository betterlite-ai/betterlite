use std::collections::BTreeMap;

use betterlite_config::spend_storage::{load_spend_storage_policy, runtime_max_prompt_length};
use serde_json::{Value, json};

use super::support::invalid;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoragePolicySpec {
    config_value: Value,
    env_value: Option<String>,
    initial_max_string_length: Option<String>,
    runtime_max_string_lengths: Vec<Option<String>>,
}

pub(super) fn storage_policy(spec: &StoragePolicySpec) -> crate::helpers::HelperResult {
    let mut environment = BTreeMap::new();
    if let Some(configured_env) = &spec.env_value {
        environment.insert(
            "STORE_PROMPTS_IN_SPEND_LOGS".to_owned(),
            configured_env.clone(),
        );
    }
    if let Some(initial) = &spec.initial_max_string_length {
        environment.insert("MAX_STRING_LENGTH_PROMPT_IN_DB".to_owned(), initial.clone());
    }
    let document = json!({"general_settings": {"store_prompts_in_spend_logs": spec.config_value}});
    let source = serde_saphyr::to_string(&document).map_err(|error| invalid(&error.to_string()))?;
    let settings = load_spend_storage_policy(&source, &environment)
        .map_err(|error| invalid(&error.to_string()))?;
    let lengths = spec
        .runtime_max_string_lengths
        .iter()
        .map(|snapshot| {
            runtime_max_prompt_length(settings.max_string_length_prompt_in_db, snapshot.as_deref())
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "store_bodies": settings.store_prompts_in_spend_logs,
        "max_string_lengths": lengths,
    }))
}
