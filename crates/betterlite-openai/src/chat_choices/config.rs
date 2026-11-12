use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use secrecy::SecretString;

use crate::chat_choices::stream::StreamingHandler;
use crate::count_tokens::counter::OpenAiTokenCounter;
use crate::count_tokens::handler::CountTokensHandler;
use crate::credentials::{ExplicitCredentials, LiteLlmGlobals, get_openai_credentials};

pub struct CredentialInputs<'input> {
    pub api_key: Option<SecretString>,
    pub litellm_api_key: Option<SecretString>,
    pub litellm_openai_key: Option<SecretString>,
    pub environment_api_key: Option<SecretString>,
    pub api_base: Option<&'input str>,
    pub litellm_api_base: Option<&'input str>,
    pub settings: &'input OpenAiCredentialsSettings,
}

#[derive(Debug)]
pub struct ChatCredentials {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

// get_api_key and get_api_base delegate to the credential resolver the OpenAI provider already owns
#[must_use]
pub fn resolve_credentials(inputs: CredentialInputs<'_>) -> ChatCredentials {
    let explicit = ExplicitCredentials {
        api_base: inputs.api_base.map(str::to_owned),
        api_key: inputs.api_key,
        organization: None,
    };
    let globals = LiteLlmGlobals {
        api_base: inputs.litellm_api_base.map(str::to_owned),
        api_key: inputs.litellm_api_key,
        openai_key: inputs.litellm_openai_key,
        organization: None,
    };
    let resolved = get_openai_credentials(
        explicit,
        globals,
        inputs.settings,
        inputs.environment_api_key,
    );
    ChatCredentials {
        api_key: resolved.api_key,
        api_base: resolved.api_base,
    }
}

// OpenAIGPTConfig's accessor surface: base_model is the identity and the stream handler carries json_mode
pub trait ChatCompletionConfig {
    fn base_model(&self, model: Option<&str>) -> Option<String> {
        model.map(str::to_owned)
    }

    fn model_response_iterator(&self, json_mode: bool) -> StreamingHandler {
        StreamingHandler { json_mode }
    }

    fn token_counter(
        &self,
        handler: CountTokensHandler,
        environment_api_key: Option<SecretString>,
    ) -> OpenAiTokenCounter {
        OpenAiTokenCounter {
            handler,
            environment_api_key,
        }
    }
}

pub struct OpenAiChatConfig;

impl ChatCompletionConfig for OpenAiChatConfig {}
