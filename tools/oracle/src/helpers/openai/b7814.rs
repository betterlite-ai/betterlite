use std::collections::BTreeMap;

use betterlite_openai::ragflow_vector_store::error::ResponseHeaders;
use betterlite_openai::vector_stores::store::CreateParams;
use serde::Deserialize;
use serde_json::Value;

mod calls;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(tag = "call", content = "input", rename_all = "snake_case")]
enum Call {
    AuthCredentials(AuthCall),
    ValidateEnvironment(EnvironmentCall),
    CompleteUrl(UrlCall),
    Endpoints,
    CreateRequest(Box<CreateCall>),
    CreateResponse(DecodeCall),
    SearchRequest,
    SearchResponse,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthCall {
    api_key: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvironmentCall {
    #[serde(default)]
    headers: BTreeMap<String, String>,
    litellm_params: Option<KeyCall>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyCall {
    api_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UrlCall {
    api_base: Option<String>,
    litellm_params: Option<BaseCall>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BaseCall {
    api_base: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCall {
    #[serde(default)]
    optional_params: CreateParams,
    api_base: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecodeCall {
    status_code: u16,
    #[serde(default)]
    headers: ResponseHeaders,
    body: String,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(Value::Array(
            input.calls.iter().map(calls::run_call).collect(),
        ))
    }
}

pub(crate) struct Op;
