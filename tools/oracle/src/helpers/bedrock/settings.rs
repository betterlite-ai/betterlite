use std::collections::BTreeMap;

use betterlite_bedrock::bedrock_settings::{
    BedrockError, ErrorHeaders, ErrorInput, ErrorReply, HeaderError, ResponseBody,
    error_response_text,
};
use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

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
    deployments: Vec<Deployment>,
    environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Deployment {
    #[serde(rename = "aws_access_key_id")]
    access_key_id: Option<String>,
    #[serde(rename = "aws_secret_access_key")]
    secret_access_key: Option<String>,
    #[serde(rename = "aws_session_token")]
    session_token: Option<String>,
    #[serde(rename = "aws_region_name")]
    region_name: Option<String>,
    #[serde(rename = "aws_session_name")]
    session_name: Option<String>,
    #[serde(rename = "aws_profile_name")]
    profile_name: Option<String>,
    #[serde(rename = "aws_role_name")]
    role_name: Option<String>,
    #[serde(rename = "aws_web_identity_token")]
    web_identity_token: Option<String>,
    #[serde(rename = "aws_sts_endpoint")]
    sts_endpoint: Option<String>,
    #[serde(rename = "aws_external_id")]
    external_id: Option<String>,
    #[serde(rename = "aws_session_tags")]
    session_tags: Option<betterlite_config::sections::bedrock_settings::SessionTagValue>,
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

#[derive(Debug, Error)]
enum Failure {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Serialize)]
struct Config {
    model_list: Vec<ModelDeployment>,
}

#[derive(Serialize)]
struct ModelDeployment {
    model_name: &'static str,
    litellm_params: Params,
}

#[derive(Serialize)]
struct Params {
    model: &'static str,
    #[serde(flatten)]
    settings: Deployment,
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
        let output = match input {
            Input::Settings(call) => settings(call),
            Input::Error(call) => error(call),
        };
        Ok(output?)
    }
}

fn settings(call: &SettingsCall) -> Result<Value, Failure> {
    let model_list = call
        .deployments
        .iter()
        .cloned()
        .map(|params| ModelDeployment {
            model_name: "oracle",
            litellm_params: Params {
                model: "bedrock/oracle",
                settings: params,
            },
        });
    let config = serde_json::to_string(&Config {
        model_list: model_list.collect(),
    })?;
    let lookup = call.environment.clone();
    let loaded = ConfigLoader { lookup }.load(&config)?;
    let deployments = &loaded.sections.bedrock_settings.deployments;
    deployments.iter().map(snapshot).collect()
}

fn snapshot(aws: &AwsSettings) -> Result<Value, Failure> {
    let secrets = [
        ("aws_access_key_id", &aws.access_key_id),
        ("aws_secret_access_key", &aws.secret_access_key),
        ("aws_session_token", &aws.session_token),
        ("aws_web_identity_token", &aws.web_identity_token),
        ("aws_bearer_token_bedrock", &aws.bearer_token),
    ];
    let texts = [
        ("aws_region_name", &aws.region_name),
        ("aws_session_name", &aws.session_name),
        ("aws_profile_name", &aws.profile_name),
        ("aws_role_name", &aws.role_name),
        ("aws_sts_endpoint", &aws.sts_endpoint),
        ("aws_external_id", &aws.external_id),
        ("aws_role_arn", &aws.role_arn),
        ("aws_web_identity_token_file", &aws.web_identity_token_file),
    ];
    let mut object = secrets
        .into_iter()
        .map(|(name, value)| (name, value.as_ref().map(ExposeSecret::expose_secret)))
        .chain(
            texts
                .into_iter()
                .map(|(name, value)| (name, value.as_deref())),
        )
        .map(|(name, value)| (name.to_owned(), value.map_or(Value::Null, Value::from)))
        .collect::<Map<_, _>>();
    let tags = serde_json::to_value(&aws.session_tags)?;
    object.insert("aws_session_tags".to_owned(), tags);
    Ok(Value::Object(object))
}

fn error(call: &ErrorCall) -> Result<Value, Failure> {
    let input = ErrorInput {
        status_code: call.status_code,
        message: call.message.clone(),
        request: call.request.clone(),
        body: call.body.clone(),
        status_code_is_synthesized: call.status_code_is_synthesized,
        headers: call.headers.clone(),
        response: call.response.as_ref().map(|reply| ErrorReply {
            response: ErrorResponse {
                status_code: reply.status_code,
                request: reply.request.clone(),
            },
            headers: reply.headers.clone(),
            body: reply.body.clone(),
        }),
    };
    let built = BedrockError::try_from(input)?;
    serialize_error(&built)
}

fn serialize_error(built: &BedrockError) -> Result<Value, Failure> {
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
    Ok(serde_json::to_value(view)?)
}

impl From<Failure> for Box<HelperError> {
    fn from(failure: Failure) -> Self {
        let class = match &failure {
            Failure::Config(_) => "ConfigError",
            Failure::Header(_) => "UnicodeEncodeError",
            Failure::Json(_) => "TypeError",
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: failure.to_string(),
        })
    }
}

mod tests;
