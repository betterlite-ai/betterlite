pub mod reply;
pub mod stream;

use betterlite_openai::openai_base_config::{CatalogCapabilities, ConfigFamily, Object};

use crate::model_policy::UNSUPPORTED_DEPLOYED_PARAMS;
use crate::model_policy::publisher::is_self_deployed_endpoint;

use super::params::{base_config, map_supported, with_token_alias};
use super::settings::PartnerSettings;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Llama3Config {
    pub settings: PartnerSettings,
}

impl Llama3Config {
    // The class narrows the inherited GPT base list to max_retries, or to the ten deployment params.
    #[must_use]
    pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
        let excluded: &[&str] = if is_self_deployed_endpoint(model) {
            &UNSUPPORTED_DEPLOYED_PARAMS
        } else {
            &["max_retries"]
        };
        base_config(ConfigFamily::Other)
            .supported_params(model, catalog)
            .into_iter()
            .filter(|param| !excluded.contains(param))
            .collect()
    }

    pub fn map_params(
        model: &str,
        requested: &Object,
        existing: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        let supported = Self::supported_params(model, catalog);
        map_supported(&supported, &with_token_alias(requested), existing);
    }
}
