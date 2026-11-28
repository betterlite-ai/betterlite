use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;

use super::embedding::EmbeddingFixture;

impl Default for EmbeddingFixture {
    fn default() -> Self {
        Self {
            input: Some(EmbeddingInput::Text(String::from("hello"))),
            optional_params: Object::new(),
            api_base: None,
            api_key: None,
            extra_headers: None,
            ssl_verify: None,
            settings: SectionSettings::default(),
        }
    }
}
