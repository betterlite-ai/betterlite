use betterlite_context::media_config_selection::image_edit::ImageEditConfigTag;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageHeaders, ImageParams};

use super::call::{ImageEditClientKind, ImageEditStep};
use super::error::ImageEditError;
use crate::api::image_input_options::options::ImageEditOptionalRequestParams;
use crate::api::image_input_options::selection::OptionalParamsPolicy;

// The call a selected provider's send receives: the mapped options, the connection controls and the branch's timeout.
pub struct ImageEditSendRequest<'send> {
    pub model: &'send str,
    pub prompt: Option<&'send str>,
    pub images: &'send [ImageFile],
    pub mask: Option<&'send ImageFile>,
    pub optional_params: &'send ImageParams,
    pub litellm_params: &'send ImageParams,
    pub extra_headers: Option<&'send ImageHeaders>,
    pub extra_body: Option<&'send ImageParams>,
    // seconds: bedrock's caller value unchanged, the other branches' timeout or REQUEST_TIMEOUT
    pub timeout: Option<f64>,
    pub api_key: Option<&'send str>,
    pub api_base: Option<&'send str>,
    pub custom_llm_provider: &'send str,
    pub client: Option<ImageEditClientKind>,
    pub asynchronous: bool,
}

// One provider's image edit configuration and send, injected at composition for each tag B157 selects.
pub trait ImageEditProvider: Send + Sync {
    fn supported_openai_params(&self, model: &str) -> Vec<String>;

    fn use_multipart_form_data(&self) -> bool;

    fn map_edit_options(
        &self,
        options: &ImageEditOptionalRequestParams,
        model: &str,
        policy: &OptionalParamsPolicy,
    ) -> Result<ImageParams, ImageEditError>;

    fn send(&self, request: ImageEditSendRequest<'_>) -> Result<ImageEditStep<'_>, ImageEditError>;
}

pub trait ImageEditProviderSource: Send + Sync {
    fn provider(&self, tag: &ImageEditConfigTag) -> Option<&dyn ImageEditProvider>;
}

// What litellm._custom_providers and custom_provider_map together say about a name
pub enum CustomEditLookup<'lookup> {
    Missing,
    Unmatched,
    Handler(&'lookup dyn CustomEditHandler),
}

// The custom handler's view of the call: kwargs go through untouched, without the named options merged in.
pub struct CustomEditRequest<'send> {
    pub model: &'send str,
    pub images: &'send [ImageFile],
    pub prompt: Option<&'send str>,
    pub api_key: Option<&'send str>,
    pub api_base: Option<&'send str>,
    pub optional_params: &'send ImageParams,
    pub timeout: Option<f64>,
    pub client: Option<ImageEditClientKind>,
}

pub trait CustomEditHandler: Send + Sync {
    fn call(
        &self,
        request: CustomEditRequest<'_>,
        asynchronous: bool,
    ) -> Result<ImageEditStep<'_>, ImageEditError>;
}

pub trait CustomEditProviders: Send + Sync {
    fn lookup(&self, provider: &str) -> CustomEditLookup<'_>;
}
