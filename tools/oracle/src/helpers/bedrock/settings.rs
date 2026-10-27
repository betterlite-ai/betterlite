use std::collections::BTreeMap;

use betterlite_bedrock::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, ResponseBody, error_response_text,
};
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::error::{ErrorRequest, ErrorResponse, ProviderErrorParts};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Settings(SettingsCall),
    Error(Box<ErrorCall>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsCall {
    deployments: Vec<Map<String, Value>>,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCall {
    status_code: u16,
    message: String,
    headers: Option<ErrorHeaders>,
    request: Option<ErrorRequest>,
    response: Option<Reply>,
    body: Option<Map<String, Value>>,
    status_code_is_synthesized: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    status_code: u16,
    request: ErrorRequest,
    headers: Vec<(String, String)>,
    body: ResponseBody,
}

#[derive(Serialize)]
struct ErrorView<'error> {
    status_code: u16,
    status_code_is_synthesized: bool,
    message: &'error str,
    headers: Option<&'error ErrorHeaders>,
    request: &'error ErrorRequest,
    response: ResponseView<'error>,
    body: Option<&'error Map<String, Value>>,
}

#[derive(Serialize)]
struct ResponseView<'error> {
    status_code: u16,
    request: &'error ErrorRequest,
    headers: &'error [(String, String)],
    text: &'error str,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Settings(call) => settings(call),
            Input::Error(call) => error(call),
        }
    }
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn settings(call: &SettingsCall) -> HelperResult {
    let model_list = call
        .deployments
        .iter()
        .map(|params| {
            let mut litellm_params = params.clone();
            litellm_params.insert("model".to_owned(), Value::from("bedrock/oracle"));
            let mut deployment = Map::new();
            deployment.insert("model_name".to_owned(), Value::from("oracle"));
            deployment.insert("litellm_params".to_owned(), Value::Object(litellm_params));
            Value::Object(deployment)
        })
        .collect();
    let mut config = Map::new();
    config.insert("model_list".to_owned(), Value::Array(model_list));
    let lookup = call.environment.clone();
    let loaded = ConfigLoader { lookup }.load(&Value::Object(config).to_string())?;
    let deployments = &loaded.sections.bedrock_settings.deployments;
    deployments.iter().map(snapshot).collect()
}

fn exposed(secret: Option<&SecretString>) -> Option<&str> {
    secret.map(ExposeSecret::expose_secret)
}

fn snapshot(aws: &AwsSettings) -> HelperResult {
    let fields = [
        ("aws_access_key_id", exposed(aws.access_key_id.as_ref())),
        ("aws_secret_access_key", exposed(aws.secret_access_key.as_ref())),
        ("aws_session_token", exposed(aws.session_token.as_ref())),
        ("aws_region_name", aws.region_name.as_deref()),
        ("aws_session_name", aws.session_name.as_deref()),
        ("aws_profile_name", aws.profile_name.as_deref()),
        ("aws_role_name", aws.role_name.as_deref()),
        ("aws_web_identity_token", exposed(aws.web_identity_token.as_ref())),
        ("aws_sts_endpoint", aws.sts_endpoint.as_deref()),
        ("aws_external_id", aws.external_id.as_deref()),
        ("aws_role_arn", aws.role_arn.as_deref()),
        ("aws_web_identity_token_file", aws.web_identity_token_file.as_deref()),
        ("aws_bearer_token_bedrock", exposed(aws.bearer_token.as_ref())),
    ];
    let mut object = fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.map_or(Value::Null, Value::from)))
        .collect::<Map<_, _>>();
    let tags = serde_json::to_value(&aws.session_tags)
        .map_err(|error| failure("TypeError", error.to_string()))?;
    object.insert("aws_session_tags".to_owned(), tags);
    Ok(Value::Object(object))
}

fn error(call: &ErrorCall) -> HelperResult {
    let base = ProviderErrorParts {
        status_code: call.status_code,
        message: call.message.clone(),
        headers: None,
        request: call.request.clone(),
        response: None,
        body: call.body.clone(),
        status_code_is_synthesized: call.status_code_is_synthesized,
    };
    let input = ErrorInput {
        base,
        headers: call.headers.clone(),
        response: call.response.as_ref().map(Reply::error_reply),
    };
    let built = BedrockError::try_from(input)
        .map_err(|error| failure("UnicodeEncodeError", error.to_string()))?;
    let view = ErrorView {
        status_code: built.error.status_code,
        status_code_is_synthesized: built.error.status_code_is_synthesized,
        message: &built.error.message,
        headers: built.headers.as_ref(),
        request: &built.error.request,
        response: ResponseView {
            status_code: built.error.response.status_code,
            request: &built.error.response.request,
            headers: &built.response_headers,
            text: error_response_text(&built.response_body),
        },
        body: built.error.body.as_ref(),
    };
    serde_json::to_value(view).map_err(|error| failure("TypeError", error.to_string()))
}

impl Reply {
    fn error_reply(&self) -> ErrorReply {
        ErrorReply {
            response: ErrorResponse {
                status_code: self.status_code,
                request: self.request.clone(),
            },
            headers: self.headers.clone(),
            body: self.body.clone(),
        }
    }
}

mod tests;
