use betterlite_azure::credentials::source::CallableTokenProvider;
use betterlite_context::media_config_selection::image_generation::ImageGenerationConfigTag;
use betterlite_context::non_default_params::mapper::OptionalParamsMapper;
use betterlite_error::dispatch::RawException;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_openai::api::image_contracts::records::ImageResponse;
use indexmap::IndexMap;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use super::call::CustomClientKind;

pub type Params = IndexMap<String, Json>;
pub type Headers = BTreeMap<String, String>;

// What a successful branch answers: the response and the model LiteLLM records in its hidden params
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchAnswer {
    pub response: ImageResponse,
    pub hidden_model: Option<String>,
}

pub struct AzureRequest<'request> {
    pub model: Option<&'request str>,
    pub prompt: &'request str,
    pub timeout: Duration,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub api_version: Option<&'request str>,
    pub azure_ad_token: Option<&'request SecretString>,
    pub azure_ad_token_provider: Option<&'request Arc<dyn CallableTokenProvider>>,
    pub optional_params: &'request Params,
    pub headers: &'request Headers,
    pub litellm_params: &'request Params,
    pub base_model: Option<&'request str>,
    pub asynchronous: bool,
    pub now_seconds: i64,
}

pub struct GenericRequest<'request> {
    pub config: ImageGenerationConfigTag,
    pub provider: &'request str,
    pub model: Option<&'request str>,
    pub prompt: &'request str,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub api_key: Option<&'request SecretString>,
    pub timeout: Duration,
}

pub struct BlackForestLabsRequest<'request> {
    pub model: &'request str,
    pub prompt: &'request str,
    pub optional_params: &'request Params,
    pub litellm_params: &'request Params,
    pub timeout: Duration,
    pub extra_headers: Option<&'request Headers>,
    pub asynchronous: bool,
}

pub struct FoundryRequest<'request> {
    pub model: Option<&'request str>,
    pub prompt: &'request str,
    pub timeout: Duration,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub api_version: Option<&'request str>,
    // LiteLLM hands the Foundry config no token of its own, so this stays None
    pub azure_ad_token: Option<&'request SecretString>,
    pub azure_ad_token_provider: Option<&'request Arc<dyn CallableTokenProvider>>,
    pub optional_params: &'request Params,
    pub headers: &'request Headers,
    pub litellm_params: &'request Params,
    pub base_model: Option<&'request str>,
    pub asynchronous: bool,
}

pub struct OpenAiRequest<'request> {
    pub model: Option<&'request str>,
    pub prompt: &'request str,
    pub timeout: Duration,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub optional_params: &'request Params,
    pub organization: Option<&'request str>,
    pub headers: &'request Headers,
    pub asynchronous: bool,
    pub now_seconds: i64,
}

pub struct BedrockRequest<'request> {
    pub model: Option<&'request str>,
    pub prompt: &'request str,
    pub timeout: Duration,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub optional_params: &'request Params,
    pub asynchronous: bool,
}

pub struct CustomRequest<'request> {
    pub provider: &'request str,
    pub model: Option<&'request str>,
    pub prompt: &'request str,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    pub optional_params: &'request Params,
    pub timeout: Duration,
    pub asynchronous: bool,
    pub client: Option<CustomClientKind>,
}

pub enum Branch<'request> {
    Azure(AzureRequest<'request>),
    Generic(GenericRequest<'request>),
    BlackForestLabs(BlackForestLabsRequest<'request>),
    Foundry(FoundryRequest<'request>),
    OpenAi(OpenAiRequest<'request>),
    Bedrock(BedrockRequest<'request>),
    Custom(CustomRequest<'request>),
}

pub struct EntraRequest<'request> {
    pub tenant_id: &'request str,
    pub client_id: &'request str,
    pub client_secret: &'request str,
    pub scope: &'request str,
}

pub struct FoundryAuthRequest<'request> {
    pub api_key: Option<&'request SecretString>,
    pub litellm_params: &'request Params,
}

// What a caller's custom handler does, so the dispatcher can select and await it without knowing its type
pub trait CustomImageHandler: Send + Sync {
    fn generate<'handler>(
        &'handler self,
        request: CustomRequest<'handler>,
        logging: &'handler mut LoggingState,
    ) -> Pin<Box<dyn Future<Output = Result<BranchAnswer, Box<RawException>>> + Send + 'handler>>;
}

#[derive(Clone)]
pub struct CustomProviderSlot {
    pub provider: String,
    pub handler: Option<Arc<dyn CustomImageHandler>>,
}

// The provider work each branch needs, supplied at the call seam: B157's config, B175's mapper, B549's auth and the sends
pub trait ImageGenerationOperations {
    fn generation_config(
        &self,
        model: Option<&str>,
        provider: &ProviderIdentity,
    ) -> Option<ImageGenerationConfigTag>;

    fn options_mapper(
        &self,
        config: &ImageGenerationConfigTag,
    ) -> Option<Box<dyn OptionalParamsMapper + '_>>;

    fn bedrock_mapper(&self) -> Option<&dyn OptionalParamsMapper>;

    fn entra_token_provider(
        &self,
        request: EntraRequest<'_>,
    ) -> Option<Arc<dyn CallableTokenProvider>>;

    fn foundry_auth_headers(
        &self,
        request: FoundryAuthRequest<'_>,
    ) -> Result<Headers, Box<RawException>>;

    fn send(
        &self,
        branch: Branch<'_>,
        logging: &mut LoggingState,
    ) -> impl Future<Output = Result<BranchAnswer, Box<RawException>>> + Send;
}
