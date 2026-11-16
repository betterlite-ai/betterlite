#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "VideoObject.error and usage are the video model's own serde_json maps, built and asserted here"
)]

pub mod credentials;
pub mod error;
pub mod params;
pub mod reply;
pub mod request;
pub mod usage;

mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// the optional-parameter map LiteLLM hands the provider: every key the caller sent stays, nulls included
pub type VideoParams = IndexMap<String, Json>;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 7] = [
    "model",
    "prompt",
    "input_reference",
    "seconds",
    "size",
    "user",
    "extra_headers",
];

pub const DEFAULT_API_BASE: &str = "https://api.dev.runwayml.com/v1";
pub const MULTIPART_FORM_DATA: bool = false;
