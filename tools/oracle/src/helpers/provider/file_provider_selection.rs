use betterlite_models::identity::provider_id::ProviderId;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::api::file_provider_selection::{
    FileProviderConfig, file_config_selection, file_timeout_capability,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    config_providers: Vec<ProviderId>,
    timeout_providers: Vec<Option<String>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let configs: Vec<Option<&'static str>> = input
            .config_providers
            .iter()
            .map(|provider| {
                file_config_selection(&input.model, *provider).map(FileProviderConfig::class_name)
            })
            .collect();
        let capabilities: Vec<bool> = input
            .timeout_providers
            .iter()
            .map(|provider| file_timeout_capability(provider.as_deref()))
            .collect();
        Ok(json!({"configs": configs, "capabilities": capabilities}))
    }
}
