use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Uuid,
    Config {
        key: String,
        user: Option<String>,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        #[serde(default)]
        config: BTreeMap<String, String>,
        default: Option<String>,
    },
    Metadata {
        #[serde(default)]
        auth: IndexMap<String, Json>,
        #[serde(default)]
        request: IndexMap<String, Json>,
        #[serde(default)]
        requester: IndexMap<String, Json>,
    },
    Payload {
        response: ResponseInput,
    },
}

#[derive(Deserialize)]
pub(crate) struct ResponseInput {
    model: Option<String>,
    #[serde(default)]
    usage: IndexMap<String, Json>,
}

fn run_uuid() -> crate::helpers::HelperResult {
    let created = betterlite_logging::opik::uuid7::create_uuid7();
    let uuid = uuid::Uuid::parse_str(&created).map_err(|error| crate::helpers::HelperError {
        class: "Uuid".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })?;
    Ok(serde_json::json!({"version": uuid.get_version_num(), "variant": 2}))
}

fn run_config(
    key: &str,
    user: Option<&str>,
    environment: &BTreeMap<String, String>,
    config: &BTreeMap<String, String>,
    default: Option<&str>,
) -> serde_json::Value {
    let value = betterlite_logging::opik::config::get_config_variable(
        key,
        user,
        environment,
        config,
        default,
    );
    serde_json::json!({"value": value})
}

fn run_metadata(
    auth: &IndexMap<String, Json>,
    request: &IndexMap<String, Json>,
    requester: &IndexMap<String, Json>,
) -> serde_json::Value {
    let mut metadata = auth.clone();
    metadata.extend(request.clone());
    metadata.extend(requester.clone());
    serde_json::json!({"metadata": metadata})
}

fn run_payload(response: &ResponseInput) -> serde_json::Value {
    let usage = betterlite_logging::opik::config::create_usage_object(&response.usage);
    let model = response.model.as_deref().unwrap_or("unknown-model");
    serde_json::json!({"type": "llm", "model": model, "usage": usage})
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Uuid => run_uuid(),
            Input::Config {
                key,
                user,
                environment,
                config,
                default,
            } => Ok(run_config(
                key,
                user.as_deref(),
                environment,
                config,
                default.as_deref(),
            )),
            Input::Metadata {
                auth,
                request,
                requester,
            } => Ok(run_metadata(auth, request, requester)),
            Input::Payload { response } => Ok(run_payload(response)),
        }
    }
}
