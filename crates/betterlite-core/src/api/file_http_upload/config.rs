use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::files::records::FileCreate;
use http::Method;
use indexmap::IndexMap;
use secrecy::SecretString;

use crate::api::vector_store_http_write::error::BaseLlmException;

use super::error::{TransformFailure, UrlFailure};
use super::plan::UploadPlan;

pub type Headers = BTreeMap<String, SecretString>;
pub type FileParams = IndexMap<String, Json>;

// the litellm_params the upload methods read directly; a transform's other inputs travel in that map
pub struct UploadParams<'params> {
    pub api_base: Option<&'params str>,
    pub api_key: Option<&'params SecretString>,
    pub ssl_verify: Option<bool>,
    pub litellm_params: &'params FileParams,
}

// httpx.Response, with the body kept as bytes; the request URL is the one the answer came from
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResponse {
    pub status_code: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub url: Option<String>,
}

// The BaseFilesConfig members create_file calls; B1015 adapts a provider config to this trait
pub trait FileUploadConfig: Send + Sync {
    fn validate_environment(&self, headers: Headers, params: &UploadParams<'_>) -> Headers;

    fn get_complete_file_url(
        &self,
        params: &UploadParams<'_>,
        data: &FileCreate,
    ) -> Result<Option<String>, UrlFailure>;

    fn file_upload_http_method(&self) -> Method {
        Method::POST
    }

    fn transform_create_file_request(
        &self,
        data: &FileCreate,
        params: &UploadParams<'_>,
    ) -> Result<UploadPlan, TransformFailure>;

    fn transform_create_file_response(
        &self,
        response: &FileResponse,
        litellm_params: &FileParams,
    ) -> Result<Json, BaseLlmException>;
}
