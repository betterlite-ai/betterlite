use crate::regional_host_dispatch::context::DispatchContext;

use super::chat::ChatFixture;

impl ChatFixture {
    pub(crate) fn context(&self) -> DispatchContext<'_> {
        DispatchContext {
            acompletion: self.acompletion,
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            custom_llm_provider: &self.provider,
            headers: self.headers.as_ref(),
            litellm_params: &self.litellm_params,
            messages: &self.messages,
            model: &self.model,
            optional_params: &self.optional_params,
            provider_config: self.provider_config,
            stream: self.stream,
            timeout: self.timeout,
            settings: &self.settings,
        }
    }
}
