use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_openai::chatgpt::headers::session::{
    LitellmParamsInput, ModelDumpOutcome, get_chatgpt_session_id,
};
use betterlite_openai::chatgpt::headers::{
    SystemPlatformEnvironment, get_chatgpt_default_headers, get_chatgpt_instructions,
    get_chatgpt_originator, get_chatgpt_user_agent,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    access_token: String,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    litellm_params: ParamsInput,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ParamsInput {
    #[default]
    Absent,
    Mapping(Map<String, Value>),
    ModelSnapshot {
        model_dump: DumpInput,
        dict: DumpInput,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DumpInput {
    #[default]
    Unavailable,
    Succeeded(Map<String, Value>),
    Failed,
}

fn to_dump(input: DumpInput) -> ModelDumpOutcome {
    match input {
        DumpInput::Unavailable => ModelDumpOutcome::Unavailable,
        DumpInput::Succeeded(fields) => ModelDumpOutcome::Succeeded(fields),
        DumpInput::Failed => ModelDumpOutcome::Failed,
    }
}

fn to_params(input: ParamsInput) -> LitellmParamsInput {
    match input {
        ParamsInput::Absent => LitellmParamsInput::Absent,
        ParamsInput::Mapping(fields) => LitellmParamsInput::Mapping(fields),
        ParamsInput::ModelSnapshot { model_dump, dict } => LitellmParamsInput::ModelSnapshot {
            model_dump: to_dump(model_dump),
            dict: to_dump(dict),
        },
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let settings = ConfigLoader {
            lookup: input.environment.clone(),
        }
        .load("")
        .map_err(|error| {
            Box::new(HelperError {
                class: "ConfigError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?
        .sections
        .chatgpt_headers;
        let platform = SystemPlatformEnvironment;
        let originator = get_chatgpt_originator(&settings);
        let user_agent = get_chatgpt_user_agent(&settings, &platform, &originator);
        let token = SecretString::from(input.access_token.clone());
        let headers = get_chatgpt_default_headers(
            &settings,
            &platform,
            &token,
            input.account_id.as_deref(),
            input.session_id.as_deref(),
        );
        let instructions = get_chatgpt_instructions(&settings);
        let params = to_params(input.litellm_params.clone());
        let session_id = get_chatgpt_session_id(&params);
        Ok(json!({
            "originator": originator,
            "user_agent": user_agent,
            "headers": headers,
            "instructions": instructions,
            "session_id": session_id,
        }))
    }
}

mod tests;
