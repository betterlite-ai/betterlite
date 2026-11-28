use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::regional_host_dispatch::context::EmbeddingContext;

#[derive(Default)]
pub(crate) struct EmbeddingFixture {
    pub(crate) input: Option<EmbeddingInput>,
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
            input: Some(EmbeddingInput::Text(String::from("hello"))),
            ..Self::default()
        }
    }

    pub(crate) fn context(&self) -> EmbeddingContext<'_> {
        EmbeddingContext {
            aembedding: false,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            extra_headers: self.extra_headers.as_ref(),
            input: self
                .input
                .as_ref()
                .expect("the embedding fixture carries an input"),
            model: "embed-model",
            optional_params: &self.optional_params,
            ssl_verify: self.ssl_verify,
            timeout: Duration::from_secs(30),
            settings: &self.settings,
        }
    }
}
