use crate::regional_host_dispatch::speech::SpeechContext;

use super::speech::SpeechFixture;

impl SpeechFixture {
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
