use betterlite_models::headers::provider_specific::{
    ProviderSpecificHeader, provider_specific_headers,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    pub(crate) entries: Vec<ProviderSpecificHeader>,
    #[serde(default)]
    pub(crate) custom_llm_provider: Option<String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        let headers =
            provider_specific_headers(Some(&input.entries), input.custom_llm_provider.as_deref());
        Ok(json!(headers))
    }
}
