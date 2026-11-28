use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_prompts::arguments::json::Object;

use super::chat::ChatFixture;
use super::user_message;

impl Default for ChatFixture {
    fn default() -> Self {
        Self {
            model: String::from("model"),
            messages: vec![user_message()],
            optional_params: Object::new(),
            litellm_params: Object::new(),
            provider: String::from("nlp_cloud"),
            api_base: None,
            api_key: None,
            headers: None,
            stream: None,
            acompletion: false,
            timeout: None,
            provider_config: None,
            settings: SectionSettings::default(),
        }
    }
}
