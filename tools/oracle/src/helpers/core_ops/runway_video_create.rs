use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_core::api::runway::tasks::settings::api_version;
use betterlite_core::api::runway::video_create::credentials::{
    VideoCredentials, validate_environment,
};
use betterlite_core::api::runway::video_create::params::map_openai_params;
use betterlite_core::api::runway::video_create::reply::create_response;
use betterlite_core::api::runway::video_create::request::create_request;
use betterlite_core::api::runway::video_create::{MULTIPART_FORM_DATA, VideoParams};
use betterlite_openai::openai_base_config::Headers;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateCall {
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    litellm_params_api_key: Option<String>,
    #[serde(default)]
    global_api_key: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    params: VideoParams,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Create {
    model: String,
    prompt: String,
    calls: Vec<CreateCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyCall {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    request_data: VideoParams,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    model: String,
    calls: Vec<ReplyCall>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", content = "params", rename_all = "snake_case")]
pub(crate) enum Input {
    Create(Create),
    Reply(Reply),
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "RunwayVideoCreateError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn credentials(call: &CreateCall) -> VideoCredentials {
    let mut found = VideoCredentials::from_environment(&call.environment);
    found.api_key = call.api_key.clone().map(SecretString::from);
    found.params_api_key = call.litellm_params_api_key.clone().map(SecretString::from);
    found.global_api_key = call.global_api_key.clone().map(SecretString::from);
    found
}

fn create_call(model: &str, prompt: &str, call: &CreateCall) -> Value {
    let found = credentials(call);
    let version = api_version(&call.environment);
    let Ok(headers) = validate_environment(&Headers::new(), &found, &version) else {
        return json!({"credentials_error": true});
    };
    let mapped = map_openai_params(&call.params);
    let form = create_request(model, prompt, call.api_base.as_deref(), &mapped);
    json!({
        "mapped": mapped,
        "url": form.url,
        "body": form.body,
        "files": [],
        "multipart": MULTIPART_FORM_DATA,
        "headers": headers,
    })
}

fn create(input: &Create) -> Value {
    let mut results = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        results.push(create_call(&input.model, &input.prompt, call));
    }
    json!({"results": results})
}

fn reply(input: &Reply) -> HelperResult {
    let mut results = Vec::with_capacity(input.calls.len());
    for call in &input.calls {
        let video = create_response(
            &input.model,
            call.body.as_bytes(),
            call.provider.as_deref(),
            &call.request_data,
        )
        .map_err(failure)?;
        results.push(json!(video));
    }
    Ok(json!({"videos": results}))
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Create(params) => Ok(create(params)),
            Input::Reply(params) => reply(params),
        }
    }
}
