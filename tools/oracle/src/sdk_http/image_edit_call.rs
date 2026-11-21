use std::collections::BTreeMap;
use std::future::Future;

use betterlite_core::api::image_input_options::options::{Quality, ResponseFormat};
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageParams};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::image_variations::Success;
use super::result::Failure;
use super::result::kinds::SdkResult;

mod native;
mod openai;
mod providers;

// One image_edit or aimage_edit call: its named arguments, the kwargs it forwards and the providers it can reach
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    #[serde(default)]
    pub asynchronous: bool,
    // every configured provider is a recorder rather than its LiteLLM send, so dispatch is observed without a network
    #[serde(default)]
    pub recorded: bool,
    pub image: Option<Images>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub mask: Option<String>,
    // the wire name LiteLLM's signature and the case files use
    #[serde(rename = "n")]
    pub count: Option<i64>,
    pub quality: Option<Quality>,
    pub response_format: Option<ResponseFormat>,
    pub size: Option<String>,
    pub user: Option<String>,
    pub extra_headers: Option<ImageParams>,
    pub extra_query: Option<ImageParams>,
    pub extra_body: Option<ImageParams>,
    pub timeout: Option<f64>,
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub kwargs: ImageParams,
    pub client: Option<Client>,
    #[serde(default)]
    pub custom_providers: Vec<CustomProvider>,
}

// LiteLLM's FileTypes as raw bytes: one file, or a list kept in order
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Images {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Client {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

// a name in litellm._custom_providers and the handlers its custom_provider_map entries carry, in map order
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CustomProvider {
    pub name: String,
    #[serde(default)]
    pub handlers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub runs: Vec<Run>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Received {
    pub model: String,
    pub images: Vec<String>,
    pub prompt: Option<String>,
    pub optional_params: ImageParams,
    pub timeout: Option<Number>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Sent {
    pub provider: String,
    #[serde(flatten)]
    pub received: Received,
    pub extra_headers: Option<ImageHeaders>,
    pub extra_body: Option<ImageParams>,
    pub asynchronous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CustomRecord {
    pub handler: String,
    #[serde(flatten)]
    pub received: Received,
    pub client: Option<Client>,
    pub asynchronous: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<Success>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<Sent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub runs: Vec<RunOutcome>,
}

impl CallKind for Call {
    fn send(
        &self,
        api_base: &str,
        env: &BTreeMap<String, String>,
    ) -> impl Future<Output = Result<SdkResult, SdkHttpError>> {
        native::execute(self, api_base, env)
    }
}
