use std::future::Future;
use std::pin::Pin;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams, ImageResponse};
use secrecy::SecretString;

use super::error::ImageVariationError;

// image_variation's signature default, which LiteLLM's get_llm_provider turns into provider=openai
pub const DEFAULT_MODEL: &str = "dall-e-2";

// the four options image_variation accepts but never forwards: the provider call always gets optional_params={}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VariationResponseFormat {
    #[default]
    Url,
    B64Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariationOptions {
    pub n: i64,
    pub response_format: VariationResponseFormat,
    pub size: Option<String>,
    pub user: Option<String>,
}

impl Default for VariationOptions {
    fn default() -> Self {
        Self {
            n: 1,
            response_format: VariationResponseFormat::Url,
            size: None,
            user: None,
        }
    }
}

// LiteLLM's kwargs after the signature defaults; async_call is set by aimage_variation and by token logging
#[derive(Debug, Clone)]
pub struct ImageVariationCall {
    pub image: ImageFile,
    // None takes the dall-e-2 default; the call never sends the model on the wire
    pub model: Option<String>,
    pub options: VariationOptions,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub custom_llm_provider: Option<String>,
    // litellm_params.timeout in seconds, passed to OpenAI unchanged and to Topaz with its fallback applied
    pub timeout: Option<f64>,
    pub async_call: bool,
    // the request context the async wrapper keeps across its executor call
    pub metadata: ImageParams,
}

pub type PendingVariation<'operation> =
    Pin<Box<dyn Future<Output = Result<ImageResponse, ImageVariationError>> + Send + 'operation>>;

// what image_variation can hand back: a provider response, a cached dict, or any other provider result
pub enum VariationStep<'operation> {
    Response(ImageResponse),
    Mapping(Json),
    Pending(PendingVariation<'operation>),
    Unexpected(Json),
}

// aimage_variation's answer: a dict from the first call was already turned into an ImageResponse,
// while a second-call result comes back exactly as the provider produced it
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariationAnswer {
    Response(ImageResponse),
    Mapping(Json),
    Unexpected(Json),
}
