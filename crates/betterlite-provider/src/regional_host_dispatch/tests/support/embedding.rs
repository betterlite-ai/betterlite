use std::collections::BTreeMap;

use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

pub(crate) struct EmbeddingFixture {
    pub(crate) input: EmbeddingInput,
    pub(crate) optional_params: Object,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) extra_headers: Option<BTreeMap<String, String>>,
    pub(crate) ssl_verify: Option<bool>,
    pub(crate) settings: SectionSettings,
}

impl EmbeddingFixture {
    pub(crate) fn new() -> Self {
        Self {
            input: EmbeddingInput::Text(String::from("hello")),
            optional_params: Object::new(),
            api_base: None,
            api_key: None,
            extra_headers: None,
            ssl_verify: None,
            settings: SectionSettings::default(),
        }
    }
}
