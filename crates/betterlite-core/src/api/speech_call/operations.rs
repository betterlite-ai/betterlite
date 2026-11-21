use std::future::Future;

use betterlite_context::media_config_selection::text_to_speech::TextToSpeechConfigTag;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::files::records::BinaryContent;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::speech::client::SuppliedClient;
use secrecy::SecretString;
use serde_json::Value;

use super::error::SpeechCallError;

// LiteLLM's three call shapes: the generic handler, the Azure OpenAI client call, and a provider dispatch
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechOperationKind {
    Generic,
    AzureOpenAi,
    Dispatch,
}

// map_openai_params receives `str | dict | None` as the voice and returns the mapped value
#[derive(Debug, Clone)]
pub struct SpeechMappingOptions {
    pub voice: Option<SpeechVoice>,
    pub drop_params: bool,
    pub kwargs: Object,
}

// one text-to-speech config's parameter mapping; the concrete configs belong to the provider owners
pub trait SpeechConfigOperation: Sync {
    fn map_openai_params(
        &self,
        model: &str,
        optional_params: Object,
        options: &SpeechMappingOptions,
    ) -> (Option<SpeechVoice>, Object);
}

// B157's selector needs the Lyria catalog answer, and its tag names the config that handles the call
pub trait SpeechConfigSource: Sync {
    fn is_lyria_model(&self, model: &str) -> bool;

    fn config(&self, tag: TextToSpeechConfigTag) -> Option<&dyn SpeechConfigOperation>;
}

// everything speech() hands one provider branch, exactly as the source passes it on
pub struct SpeechOperation<'service> {
    pub kind: SpeechOperationKind,
    pub provider: String,
    pub config: Option<&'service dyn SpeechConfigOperation>,
    pub config_tag: Option<TextToSpeechConfigTag>,
    pub model: String,
    pub input: String,
    pub voice: Option<SpeechVoice>,
    pub optional_params: Object,
    pub litellm_params: Object,
    pub timeout_seconds: f64,
    pub extra_headers: Option<Object>,
    pub api_base: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_version: Option<String>,
    pub organization: Option<String>,
    pub max_retries: u32,
    pub azure_ad_token: Option<SecretString>,
    pub azure_ad_token_provider: Option<Value>,
    pub client: Option<SuppliedClient>,
    pub aspeech: bool,
    pub kwargs: Object,
}

// the B010 operation the downstream provider owners implement; this task never imports those adapters
pub trait SpeechOperations: Sync {
    fn execute(
        &self,
        operation: SpeechOperation<'_>,
    ) -> impl Future<Output = Result<BinaryContent, SpeechCallError>> + Send;
}
