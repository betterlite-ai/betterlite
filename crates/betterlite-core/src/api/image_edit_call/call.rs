use std::future::Future;
use std::pin::Pin;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams, ImageResponse};

use super::error::ImageEditError;
use crate::api::image_input_options::options::Quality;
use crate::api::image_input_options::options::ResponseFormat;

// image_edit's signature default, which LiteLLM's get_llm_provider turns into provider=openai
pub const DEFAULT_MODEL: &str = "dall-e-2";

// LiteLLM's constants.request_timeout default for the generic, Stability and BFL edit handlers
pub const DEFAULT_REQUEST_TIMEOUT: f64 = 6000.0;

// flatten_form_field_values recurses at most this deep
pub const MAX_FORM_DEPTH: usize = 100;

// image_edit's filter_out_litellm_params excluding list, in LiteLLM's order
pub const OPENAI_EDIT_PARAMS: [&str; 16] = [
    "user",
    "request_timeout",
    "api_base",
    "api_version",
    "api_key",
    "deployment_id",
    "organization",
    "base_url",
    "default_headers",
    "timeout",
    "max_retries",
    "n",
    "quality",
    "size",
    "style",
    "async_call",
];

// image_edit's image argument: None becomes [], a single value a one-element list, and a list keeps its order
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ImageInput {
    #[default]
    Absent,
    One(Box<ImageFile>),
    Many(Box<[ImageFile]>),
}

impl ImageInput {
    #[must_use]
    pub fn images(&self) -> Vec<ImageFile> {
        match self {
            Self::Absent => Vec::new(),
            Self::One(image) => vec![image.as_ref().clone()],
            Self::Many(images) => images.to_vec(),
        }
    }
}

// the client class a custom handler is offered; the wrong class arrives as None
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageEditClientKind {
    Sync,
    Asynchronous,
}

impl ImageEditClientKind {
    #[must_use]
    pub const fn of(asynchronous: bool) -> Self {
        if asynchronous {
            Self::Asynchronous
        } else {
            Self::Sync
        }
    }
}

// image_edit's named parameters and the kwargs map LiteLLM keeps for its handlers and generic params
#[derive(Debug, Clone, Default)]
pub struct ImageEditCall {
    pub image: ImageInput,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub mask: Option<ImageFile>,
    // the wire name LiteLLM's signature uses for this count
    pub count: Option<i64>,
    pub quality: Option<Quality>,
    pub response_format: Option<ResponseFormat>,
    pub size: Option<String>,
    pub user: Option<String>,
    pub extra_headers: Option<ImageParams>,
    // accepted and never forwarded, exactly as LiteLLM ignores it
    pub extra_query: Option<ImageParams>,
    pub extra_body: Option<ImageParams>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<String>,
    pub kwargs: ImageParams,
    pub client: Option<ImageEditClientKind>,
}

// kwargs.pop("async_call", False) is True: only a boolean true counts, and the entry call consumes it
#[must_use]
pub fn split_async_call(kwargs: &ImageParams) -> (bool, ImageParams) {
    let mut remaining = kwargs.clone();
    let popped = remaining.shift_remove("async_call");
    (matches!(popped, Some(Json::Bool(true))), remaining)
}

// timeout or DEFAULT_REQUEST_TIMEOUT: a missing or zero timeout takes the configured value
pub fn request_timeout(timeout: Option<f64>, fallback_seconds: f64) -> Result<f64, ImageEditError> {
    match timeout {
        Some(seconds) if seconds != 0.0 => seconds_checked(seconds),
        Some(_) | None => seconds_checked(fallback_seconds),
    }
}

pub(crate) fn seconds_checked(seconds: f64) -> Result<f64, ImageEditError> {
    if seconds.is_finite() && seconds >= 0.0 {
        Ok(seconds)
    } else {
        Err(ImageEditError::InvalidTimeout { seconds })
    }
}

pub type PendingEdit<'operation> =
    Pin<Box<dyn Future<Output = Result<ImageResponse, ImageEditError>> + Send + 'operation>>;

// what image_edit can hand back: a ready handler answer, or the coroutine an async provider returned
pub enum ImageEditStep<'operation> {
    Answer(ImageResponse),
    Pending(PendingEdit<'operation>),
}
