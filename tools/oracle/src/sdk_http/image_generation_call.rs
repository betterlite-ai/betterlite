use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::Failure;
use super::result::kinds::SdkResult;

mod native;
mod operations;
mod report;
mod sends;

pub(super) const MOCK: &str = "{mock}";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub model: Option<String>,
    pub prompt: String,
    pub custom_llm_provider: Option<String>,
    #[serde(rename = "n")]
    pub image_count: Option<i64>,
    pub quality: Option<String>,
    pub response_format: Option<String>,
    pub size: Option<String>,
    pub style: Option<String>,
    pub user: Option<String>,
    pub timeout: Option<f64>,
    pub api_key: Option<String>,
    pub api_version: Option<String>,
    #[serde(default)]
    pub aimg_generation: bool,
    pub mock_response: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub organization: Option<String>,
    pub base_model: Option<String>,
    #[serde(default)]
    pub custom_providers: Vec<String>,
    #[serde(default)]
    pub litellm_settings: Settings,
}

// What the SDK call answers: the ImageResponse, or the exception the SDK raises for this class
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Failure>,
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
