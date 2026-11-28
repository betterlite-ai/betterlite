use std::time::Duration;

use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::api::speech_provider_contract::SpeechHeaders;

pub(crate) struct SpeechFixture {
    pub(crate) input: String,
    pub(crate) model: String,
    pub(crate) voice: Option<SpeechVoice>,
    pub(crate) optional_params: Object,
    pub(crate) litellm_params: Object,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) extra_headers: Option<SpeechHeaders>,
    pub(crate) ssl_verify: Option<bool>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) settings: SectionSettings,
}

impl SpeechFixture {
    pub(crate) fn new() -> Self {
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
