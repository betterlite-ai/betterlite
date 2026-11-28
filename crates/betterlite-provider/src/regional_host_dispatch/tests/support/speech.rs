use std::time::Duration;

use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;

use crate::api::speech_provider_contract::SpeechHeaders;
use crate::regional_host_dispatch::speech::SpeechContext;

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
    // field_reassign_with_default rejects a Default value followed by field assignment, so tests start here
    pub(crate) fn baseline() -> Self {
        Self::default()
    }

    pub(crate) fn context(&self) -> SpeechContext<'_> {
        SpeechContext {
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            extra_headers: self.extra_headers.as_ref(),
            input: &self.input,
            litellm_params: &self.litellm_params,
            model: &self.model,
            optional_params: &self.optional_params,
            ssl_verify: self.ssl_verify,
            timeout: self.timeout,
            voice: self.voice.clone(),
            settings: &self.settings,
        }
    }
}
