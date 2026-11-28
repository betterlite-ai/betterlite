use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::regional_host_keys::Settings as SectionSettings;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use secrecy::SecretString;

use crate::regional_host_dispatch::context::DispatchContext;

use super::user_message;

pub(crate) struct ChatFixture {
    pub(crate) model: String,
    pub(crate) messages: Vec<Message>,
    pub(crate) optional_params: Object,
    pub(crate) litellm_params: Object,
    pub(crate) provider: String,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) headers: Option<BTreeMap<String, String>>,
    pub(crate) stream: Option<bool>,
    pub(crate) acompletion: bool,
    pub(crate) timeout: Option<Duration>,
    pub(crate) provider_config: Option<ChatConfigTag>,
    pub(crate) settings: SectionSettings,
}

impl ChatFixture {
    pub(crate) fn new() -> Self {
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
