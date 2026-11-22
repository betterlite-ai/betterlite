use std::str::FromStr as _;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection, file_timeout_capability,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    config_providers: Vec<String>,
    timeout_providers: Vec<Option<String>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let configs: Vec<Option<&'static str>> = input
            .config_providers
            .iter()
            .map(|provider| config_class(&input.model, provider))
            .collect::<Result<_, _>>()?;
        let capabilities: Vec<bool> = input
            .timeout_providers
            .iter()
            .map(|provider| file_timeout_capability(provider.as_deref()))
            .collect();
        Ok(json!({"configs": configs, "capabilities": capabilities}))
    }
}

fn config_class(model: &str, provider: &str) -> Result<Option<&'static str>, Box<HelperError>> {
    let id = ProviderId::from_str(provider).map_err(|_invalid| {
        Box::new(HelperError {
            class: String::from("ValueError"),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("'{provider}' is not a valid LlmProviders"),
        })
    })?;
    Ok(file_config_selection(model, id).map(FileProviderConfig::class_name))
}
