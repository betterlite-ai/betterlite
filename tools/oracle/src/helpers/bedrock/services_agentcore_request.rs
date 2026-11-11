use std::collections::BTreeMap;

use betterlite_bedrock::agentcore_request::error::AgentCoreRequestError;
use betterlite_bedrock::agentcore_request::message::AgentCoreMessage;
use betterlite_bedrock::agentcore_request::payload::transform_request;
use betterlite_bedrock::agentcore_request::policy::{
    HAS_CUSTOM_STREAM_WRAPPER, SHOULD_FAKE_STREAM, SUPPORTED_OPENAI_PARAMS,
    SUPPORTS_STREAM_PARAM_IN_BODY, get_error_class,
};
use betterlite_bedrock::agentcore_request::url::complete_url;
use betterlite_bedrock::bedrock_settings::ErrorHeaders;
use betterlite_bedrock::model_routes::ParamMap;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

mod signing;
mod tests;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(run_scenario(input))
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "scenario", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    RuntimeArnRequired {
        models: Vec<String>,
    },
    ArnRegion {
        runs: Vec<UrlRun>,
    },
    RuntimeSessionId {
        runs: Vec<TransformRun>,
    },
    RuntimeUserId {
        runs: Vec<TransformRun>,
    },
    MultimodalForwarding {
        runs: Vec<TransformRun>,
    },
    QualifierQuery {
        runs: Vec<UrlRun>,
    },
    StreamBodyPolicy {
        optional_params: IndexMap<String, Json>,
        headers: Vec<(String, String)>,
        error: ErrorRun,
    },
    AgentcoreBearerSigning {
        runs: Vec<SignRun>,
    },
    AgentcoreIamSigning {
        runs: Vec<SignRun>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UrlRun {
    model: String,
    api_base: Option<String>,
    #[serde(default)]
    optional_params: ParamMap,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransformRun {
    model: String,
    messages: Vec<AgentCoreMessage>,
    #[serde(default)]
    optional_params: ParamMap,
    #[serde(default)]
    litellm_params: ParamMap,
    #[serde(default)]
    headers: Vec<(String, String)>,
    uuid: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorRun {
    status: u16,
    message: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignRun {
    model: String,
    api_base: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    #[serde(default)]
    optional_params: ParamMap,
    request_data: IndexMap<String, Json>,
    api_key: Option<String>,
    configured_bearer: Option<String>,
    credentials: CredentialRun,
    now: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialRun {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
}

fn run_scenario(input: &Input) -> Value {
    match input {
        Input::RuntimeArnRequired { models } => json!({
            "results": models
                .iter()
                .map(|model| url_result(&UrlRun {
                    model: model.clone(),
                    api_base: None,
                    optional_params: ParamMap::new(),
                }))
                .collect::<Vec<_>>(),
        }),
        Input::ArnRegion { runs } | Input::QualifierQuery { runs } => json!({
            "results": runs.iter().map(url_result).collect::<Vec<_>>(),
        }),
        Input::RuntimeSessionId { runs }
        | Input::RuntimeUserId { runs }
        | Input::MultimodalForwarding { runs } => json!({
            "results": runs.iter().map(transform_result).collect::<Vec<_>>(),
        }),
        Input::StreamBodyPolicy {
            optional_params,
            headers,
            error,
        } => json!({
            "supported": SUPPORTED_OPENAI_PARAMS,
            "mapped": optional_params,
            "has_custom_stream_wrapper": HAS_CUSTOM_STREAM_WRAPPER,
            "supports_stream_param_in_request_body": SUPPORTS_STREAM_PARAM_IN_BODY,
            "should_fake_stream": SHOULD_FAKE_STREAM,
            "headers": header_map(headers),
            "error": error_result(error),
        }),
        Input::AgentcoreBearerSigning { runs } | Input::AgentcoreIamSigning { runs } => json!({
            "results": runs.iter().map(signing::sign_result).collect::<Vec<_>>(),
        }),
    }
}

fn url_result(run: &UrlRun) -> Value {
    match complete_url(&run.model, &run.optional_params) {
        Ok(url) => json!({"api_base": run.api_base, "ok": true, "url": url}),
        Err(error) => json!({
            "api_base": run.api_base,
            "ok": false,
            "error": error_kind(&error),
        }),
    }
}

fn transform_result(run: &TransformRun) -> Value {
    let Ok(uuid) = Uuid::parse_str(&run.uuid) else {
        return json!({"ok": false, "error": "invalid_uuid"});
    };
    let mut headers = optional_headers(&run.headers);
    match transform_request(
        &run.messages,
        &run.optional_params,
        &run.litellm_params,
        &mut headers,
        uuid,
    ) {
        Ok(payload) => json!({
            "ok": true,
            "model": run.model,
            "payload": payload,
            "headers": present_header_map(&headers),
        }),
        Err(error) => json!({"ok": false, "model": run.model, "error": error_kind(&error)}),
    }
}

fn error_result(run: &ErrorRun) -> Value {
    let headers = ErrorHeaders::Http(run.headers.clone());
    match get_error_class(run.message.clone(), run.status, Some(headers)) {
        Ok(error) => json!({
            "status_code": error.error.status_code,
            "status_code_is_synthesized": error.error.status_code_is_synthesized,
            "headers": header_map(&error.response_headers),
            "response_status": error.error.response.status_code,
        }),
        Err(error) => json!({"class": format!("{error:?}")}),
    }
}

pub(super) fn error_kind(error: &AgentCoreRequestError) -> String {
    match error {
        AgentCoreRequestError::InvalidModelFormat => "invalid_model_format".to_owned(),
        AgentCoreRequestError::InvalidArn { .. } => "invalid_arn".to_owned(),
        AgentCoreRequestError::EmptyMessages => "empty_messages".to_owned(),
        AgentCoreRequestError::WrongType { field } => format!("wrong_type:{field}"),
        AgentCoreRequestError::Parameters { .. } => "parameters".to_owned(),
        AgentCoreRequestError::Signing { .. } => "signing".to_owned(),
    }
}

pub(super) fn optional_headers(pairs: &[(String, String)]) -> Vec<(String, Option<String>)> {
    pairs
        .iter()
        .map(|(name, value)| (name.clone(), Some(value.clone())))
        .collect()
}

pub(super) fn header_map(pairs: &[(String, String)]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (name, value) in pairs {
        map.insert(name.clone(), value.clone());
    }
    map
}

fn present_header_map(pairs: &[(String, Option<String>)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), value.clone()?)))
        .collect()
}
