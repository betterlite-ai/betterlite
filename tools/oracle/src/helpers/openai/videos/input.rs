use std::collections::BTreeMap;

use betterlite_openai::videos::create::VideoCreate;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Call {
    Create(Create),
    CreateResponse(CreateResponse),
    ContentRequest(ContentRequest),
    ContentResponse(Body),
    RemixRequest(RemixRequest),
    RemixResponse(Response),
    ListRequest(ListRequest),
    ListResponse(Response),
    DeleteRequest(Request),
    DeleteResponse(Body),
    StatusRequest(Request),
    StatusResponse(Response),
    ValidateEnvironment(Environment),
    CompleteUrl(CompleteUrl),
    ErrorClass(ErrorCall),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Reference {
    Text { text: String },
    Bytes { data_base64: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Create {
    pub(crate) model: String,
    pub(crate) prompt: String,
    pub(crate) api_base: String,
    pub(crate) options: VideoCreate,
    pub(crate) reference: Option<Reference>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateResponse {
    pub(crate) model: String,
    pub(crate) provider: Option<String>,
    pub(crate) body_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Response {
    pub(crate) provider: Option<String>,
    pub(crate) body_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Body {
    pub(crate) body_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(crate) video_id: String,
    pub(crate) api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentRequest {
    pub(crate) video_id: String,
    pub(crate) api_base: String,
    pub(crate) variant: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemixRequest {
    pub(crate) video_id: String,
    pub(crate) prompt: String,
    pub(crate) api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListRequest {
    pub(crate) api_base: String,
    pub(crate) after: Option<String>,
    pub(crate) limit: Option<i64>,
    pub(crate) order: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Environment {
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) api_key: Option<String>,
    pub(crate) params_api_key: Option<String>,
    pub(crate) global_api_key: Option<String>,
    pub(crate) openai_key: Option<String>,
    pub(crate) environment_api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompleteUrl {
    pub(crate) api_base: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCall {
    pub(crate) message: String,
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
}
