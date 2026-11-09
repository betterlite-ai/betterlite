use serde_json::Value;

use crate::openai_base_config::{CatalogCapabilities, Object, OpenAiConfig};

const EXTRA_BODY: &str = "extra_body";
const PROVIDER_OPTIONS: &str = "providerOptions";

#[derive(Debug, Clone)]
pub struct VercelAiGatewayConfig {
    pub base: OpenAiConfig,
}

impl VercelAiGatewayConfig {
    pub const PROVIDER: &'static str = "vercel_ai_gateway";

    #[must_use]
    pub fn supported_params(
        &self,
        model: &str,
        catalog: &CatalogCapabilities,
    ) -> Vec<&'static str> {
        let mut params = self.base.supported_params(model, catalog);
        if !params.contains(&EXTRA_BODY) {
            params.push(EXTRA_BODY);
        }
        params
    }

    pub fn map_openai_params(
        &self,
        model: &str,
        non_default_params: &mut Object,
        optional_params: &mut Object,
        catalog: &CatalogCapabilities,
    ) {
        self.base
            .map_params(model, non_default_params, optional_params, catalog);
        let mut extra_body = Object::new();
        if let Some(value) = non_default_params
            .remove(PROVIDER_OPTIONS)
            .filter(|value| !value.is_null())
        {
            extra_body.insert(String::from(PROVIDER_OPTIONS), value);
        }
        optional_params.insert(String::from(EXTRA_BODY), Value::Object(extra_body));
    }
}
