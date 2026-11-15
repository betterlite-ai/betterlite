use betterlite_openai::openai_base_config::{CatalogCapabilities, ConfigFamily, Object};

use super::params::{base_config, with_token_alias};
use super::settings::PartnerSettings;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ai21Config {
    pub settings: PartnerSettings,
}

impl Ai21Config {
    // The class defines no supported list of its own, so validation sees the inherited GPT base.
    #[must_use]
    pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
        base_config(ConfigFamily::Other).supported_params(model, catalog)
    }

    // The alias applies first, then litellm.OpenAIConfig adds reasoning_effort for an unknown model.
    pub fn map_params(
        model: &str,
        requested: &Object,
        existing: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        base_config(ConfigFamily::OpenAi).map_params(
            model,
            &with_token_alias(requested),
            existing,
            catalog,
        );
    }
}
