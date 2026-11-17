use betterlite_config::sections::self_hosted_dispatch::Settings as SectionSettings;
use secrecy::SecretString;

// the config section's values, before the dispatch borrows them for one call
#[derive(Default)]
pub(super) struct SectionFixture {
    pub(super) hosted_vllm_api_base: Option<String>,
    pub(super) hosted_vllm_api_key: Option<String>,
    pub(super) ollama_api_base: Option<String>,
    pub(super) ollama_api_key: Option<String>,
    pub(super) lemonade_api_key: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) ollama_key: Option<String>,
    pub(super) lemonade_key: Option<String>,
}

impl SectionFixture {
    pub(super) fn hosted_vllm_api_base(mut self, value: Option<&str>) -> Self {
        self.hosted_vllm_api_base = value.map(str::to_owned);
        self
    }

    pub(super) fn hosted_vllm_api_key(mut self, value: Option<&str>) -> Self {
        self.hosted_vllm_api_key = value.map(str::to_owned);
        self
    }

    pub(super) fn ollama_api_base(mut self, value: Option<&str>) -> Self {
        self.ollama_api_base = value.map(str::to_owned);
        self
    }

    pub(super) fn ollama_api_key(mut self, value: Option<&str>) -> Self {
        self.ollama_api_key = value.map(str::to_owned);
        self
    }

    pub(super) fn lemonade_api_key(mut self, value: Option<&str>) -> Self {
        self.lemonade_api_key = value.map(str::to_owned);
        self
    }

    pub(super) fn api_base(mut self, value: Option<&str>) -> Self {
        self.api_base = value.map(str::to_owned);
        self
    }

    pub(super) fn api_key(mut self, value: Option<&str>) -> Self {
        self.api_key = value.map(str::to_owned);
        self
    }

    pub(super) fn ollama_key(mut self, value: Option<&str>) -> Self {
        self.ollama_key = value.map(str::to_owned);
        self
    }

    pub(super) fn lemonade_key(mut self, value: Option<&str>) -> Self {
        self.lemonade_key = value.map(str::to_owned);
        self
    }

    pub(super) fn into_settings(self) -> SectionSettings {
        SectionSettings {
            hosted_vllm_api_base: self.hosted_vllm_api_base,
            hosted_vllm_api_key: self.hosted_vllm_api_key.map(SecretString::from),
            ollama_api_base: self.ollama_api_base,
            ollama_api_key: self.ollama_api_key.map(SecretString::from),
            lemonade_api_key: self.lemonade_api_key.map(SecretString::from),
            api_base: self.api_base,
            api_key: self.api_key.map(SecretString::from),
            ollama_key: self.ollama_key.map(SecretString::from),
            lemonade_key: self.lemonade_key.map(SecretString::from),
        }
    }
}
