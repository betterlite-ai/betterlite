use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

#[must_use]
pub fn supported_params(model: &str, catalog: &CatalogCapabilities) -> Vec<&'static str> {
    let mut params = OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
    .supported_params(model, catalog);
    params.push("audio");
    params
}

pub fn map_params(
    model: &str,
    non_default_params: &mut Object,
    optional_params: &mut Object,
    catalog: &CatalogCapabilities,
) {
    OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
    .map_params(model, non_default_params, optional_params, catalog);
}
