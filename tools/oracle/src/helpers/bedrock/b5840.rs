use std::collections::BTreeMap;
use std::future::ready;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::sagemaker_runtime::endpoint::{EndpointRequest, ParamMap, runtime_url};
use betterlite_bedrock::signing::request::AwsSignedRequest;
use betterlite_bedrock::signing::service::{ServiceName, ServiceRequest, sign_service_request};
use betterlite_bedrock::signing::target::RegionInputs;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::sagemaker_chat::request;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_prompts::arguments::json::Json as PromptJson;
use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::value::to_raw_value;
use serde_json::{Value, json};
use tokio::runtime::Builder;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Url(case) => Ok(url_result(case)),
            Input::ComponentHeader(case) => Ok(component_result(case)),
            Input::SignedRequest(case) => signed_result(case),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Url(UrlInput),
    ComponentHeader(ComponentHeaderInput),
    SignedRequest(SignedRequestInput),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlInput {
    model: String,
    region: String,
    base_url: Option<String>,
    stream: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ComponentHeaderInput {
    model_id: Option<String>,
    headers: Headers,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedRequestInput {
    api_base: String,
    model: String,
    headers: Headers,
    optional_params: IndexMap<String, Json>,
    request_data: IndexMap<String, Json>,
    credentials: CredentialInput,
    now: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialInput {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

fn url_result(case: &UrlInput) -> Value {
    json!({
        "url": runtime_url(&EndpointRequest {
            model: &case.model,
            region: &case.region,
            base_url: case.base_url.as_deref(),
            stream: case.stream,
        }),
    })
}

fn component_result(case: &ComponentHeaderInput) -> Value {
    let mut optional_params = Object::new();
    if let Some(component) = case.model_id.as_ref() {
        optional_params.insert(
            String::from("model_id"),
            PromptJson::Text(component.clone()),
        );
    }
    json!({"headers": request::validate_environment(case.headers.clone(), &optional_params)})
}

// SagemakerChatConfig.sign_request drops the api_key it is handed and always signs with SigV4
fn signed_result(case: &SignedRequestInput) -> HelperResult {
    let keys = &case.credentials;
    let credentials = Credentials {
        access_key_id: SecretString::from(keys.access_key_id.clone()),
        secret_access_key: SecretString::from(keys.secret_access_key.clone()),
        session_token: keys.session_token.clone().map(SecretString::from),
    };
    let pairs: Vec<(String, Option<String>)> = case
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect();
    let params = raw_params(&case.optional_params)?;
    let stamp = UNIX_EPOCH
        .checked_add(Duration::from_secs(case.now))
        .unwrap_or(UNIX_EPOCH);
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return Ok(json!({"ok": false, "error": error.to_string()})),
    };
    let signed = runtime.block_on(sign_service_request(
        ServiceRequest {
            service: ServiceName::Sagemaker,
            headers: &pairs,
            optional_params: &params,
            request_data: &case.request_data,
            api_base: &case.api_base,
            model: Some(&case.model),
            api_key: None,
            configured_bearer: None,
            region_inputs: RegionInputs::default(),
            now: stamp,
        },
        move |_settings, _region| ready(Ok(credentials)),
    ));
    Ok(signed.map_or_else(
        |error| json!({"ok": false, "error": error.to_string()}),
        |request| signed_json(&request),
    ))
}

fn raw_params(params: &IndexMap<String, Json>) -> Result<ParamMap, Box<HelperError>> {
    params
        .iter()
        .map(|(name, value)| {
            to_raw_value(value)
                .map(|raw| (name.clone(), raw))
                .or_value_error()
        })
        .collect()
}

fn signed_json(request: &AwsSignedRequest) -> Value {
    let Ok(body) = String::from_utf8(request.body.clone()) else {
        return json!({"ok": false, "error": "the signed body is not UTF-8"});
    };
    json!({
        "ok": true,
        "method": request.method,
        "url": request.url,
        "headers": request.headers.iter().cloned().collect::<BTreeMap<_, _>>(),
        "body": body,
    })
}
