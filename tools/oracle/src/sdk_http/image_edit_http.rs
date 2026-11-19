use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::{ImageFile, ImageParams, ImageResponse};
use betterlite_provider::api::image_edit_http::service::ClientKind;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::call_kind::CallKind;
use super::error::SdkHttpError;
use super::result::kinds::SdkResult;

mod native;
mod support;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

impl Mode {
    const fn client_kind(self) -> ClientKind {
        match self {
            Self::Sync => ClientKind::Sync,
            Self::Asynchronous => ClientKind::Asynchronous,
        }
    }
}

// The uploaded image as the wire can carry it: the transform's content type or the one its file name implies
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageInput {
    pub filename: String,
    pub content: String,
    #[serde(default)]
    pub content_type: Option<String>,
}

impl ImageInput {
    fn file(&self) -> ImageFile {
        ImageFile {
            filename: Some(self.filename.clone()),
            content: self.content.clone().into_bytes(),
            content_type: self.content_type.clone(),
        }
    }
}

// The handler's inputs: the mode and model, the transform's fields, the caller's headers and the arguments the handler ignores
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Call {
    pub mode: Mode,
    pub model: String,
    #[serde(default)]
    pub prompt: Option<String>,
    pub image: ImageInput,
    #[serde(default)]
    pub optional_params: ImageParams,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub ssl_verify: Option<bool>,
    #[serde(default)]
    pub extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub extra_body: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub fake_stream: bool,
    #[serde(default)]
    pub litellm_metadata: Option<IndexMap<String, Json>>,
    #[serde(default = "multipart_default")]
    pub multipart: bool,
    pub timeout_seconds: u64,
}

const fn multipart_default() -> bool {
    true
}

// The exception _handle_error raises, as its callers can observe it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    pub status: u16,
    pub synthesized: bool,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}

// The pre-call the handler made, projected so a masked credential never decides the comparison
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggedCall {
    pub input: Json,
    pub complete_input_dict: Json,
    pub header_names: Vec<String>,
    pub api_base_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub result: Option<ImageResponse>,
    pub provider_headers: Option<BTreeMap<String, String>>,
    pub failure: Option<Failure>,
    pub logged: Option<LoggedCall>,
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
