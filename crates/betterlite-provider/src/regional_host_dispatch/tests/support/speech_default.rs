use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_prompts::arguments::json::Object;

use super::speech::SpeechFixture;

impl Default for SpeechFixture {
    fn default() -> Self {
        Self {
            input: String::from("hello"),
            model: String::from("speech-02-hd"),
            voice: None,
            optional_params: Object::new(),
            litellm_params: Object::new(),
            api_base: None,
            api_key: None,
            extra_headers: None,
            ssl_verify: None,
            timeout: None,
            settings: SectionSettings::default(),
        }
    }
}
