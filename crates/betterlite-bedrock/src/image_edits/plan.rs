use std::time::{Duration, SystemTime};

use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::api::image_contracts::records::ImageParams;
use secrecy::SecretString;

use crate::model_routes::ParamMap;
use crate::signing::request::AwsSignedRequest;

#[derive(Debug, Clone, Copy, Default)]
pub struct EditEnvironment<'text> {
    pub region_name: Option<&'text str>,
    pub region: Option<&'text str>,
    pub session_region: Option<&'text str>,
    pub runtime_endpoint: Option<&'text str>,
    pub bearer_token: Option<&'text SecretString>,
}

// A Python image value an edit caller hands over: bytes already read, or a base64 string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditImage {
    Bytes(Vec<u8>),
    Text(String),
}

pub struct EditCall<'text> {
    pub model: &'text str,
    pub prompt: Option<&'text str>,
    pub images: &'text [EditImage],
    pub optional_params: ImageParams,
    pub api_base: Option<&'text str>,
    pub extra_headers: Option<&'text [(String, String)]>,
    pub api_key: Option<&'text SecretString>,
    pub timeout: Option<Duration>,
    pub aimage_edit: bool,
    pub environment: EditEnvironment<'text>,
    pub now: SystemTime,
}

// The raw model-cost map and catalog resolution B635's selector reads for every model.
#[derive(Clone, Copy)]
pub struct EditPlan<'cost, 'context> {
    pub model_cost: &'cost ParamMap,
    pub resolution: &'context ResolutionContext<'context>,
}

pub struct PreparedRequest {
    pub endpoint_url: String,
    pub signed: AwsSignedRequest,
    pub data: ImageParams,
}
