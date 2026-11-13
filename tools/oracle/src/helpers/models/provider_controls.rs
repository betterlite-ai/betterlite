use std::collections::BTreeMap;

use betterlite_models::controls::call_params::{
    CallParams, RequestCall, TRUSTED_CREDENTIALS_KEY, TrustedCall, TrustedCredentials, call_params,
};
use betterlite_models::controls::control_options::{ControlOptions, parse_control_options};
use betterlite_models::controls::extra_body::check_extra_body;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::controls::optional_params::OptionalParams;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    Parse(ParseInput),
    TrustedCredentials(TrustedCredentialsInput),
    ExtraBody(ExtraBodyInput),
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ParseInput {
    #[serde(default)]
    pub(crate) stream_chunk_size: Option<JsonValue>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct TrustedCredentialsInput {
    #[serde(default)]
    pub(crate) trusted: TrustedProjection,
    #[serde(default)]
    pub(crate) credentials: BTreeMap<String, SecretString>,
    #[serde(default)]
    pub(crate) scenarios: Vec<Scenario>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct TrustedProjection {
    pub(crate) model: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) api_base: Option<String>,
    pub(crate) custom_llm_provider: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Scenario {
    pub(crate) trusted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ExtraBodyInput {
    #[serde(default)]
    pub(crate) body: Option<JsonValue>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> crate::helpers::HelperResult {
        match input {
            Input::Parse(fields) => Ok(parse(fields)),
            Input::TrustedCredentials(fields) => Ok(trusted_credentials(fields)),
            Input::ExtraBody(fields) => extra_body(fields),
        }
    }
}

fn parse(fields: &ParseInput) -> serde_json::Value {
    match parse_control_options(fields.stream_chunk_size.as_ref()) {
        Ok(options) => json!({
            "valid": true,
            "stream_chunk_size": options.stream_chunk_size,
        }),
        Err(error) => json!({"valid": false, "param": error.param()}),
    }
}

fn trusted_credentials(fields: &TrustedCredentialsInput) -> serde_json::Value {
    let mut results = Vec::new();
    for scenario in &fields.scenarios {
        let credentials = scenario.trusted.then(|| TrustedCredentials {
            values: fields.credentials.clone(),
        });
        let trusted = TrustedCall {
            model: fields.trusted.model.clone(),
            api_key: fields.trusted.api_key.clone(),
            api_base: fields.trusted.api_base.clone(),
            custom_llm_provider: fields.trusted.custom_llm_provider.clone(),
            control: ControlOptions::default(),
            credentials,
            drop_params: None,
            no_log: None,
            optional: OptionalParams::default(),
        };
        let params = call_params(&trusted, &RequestCall::default());
        let credential_keys = params
            .credentials
            .as_ref()
            .map(|carried| carried.values.keys().cloned().collect::<Vec<String>>());
        results.push(json!({
            "trusted": scenario.trusted,
            "keys": carried_keys(&params),
            "credential_keys": credential_keys,
        }));
    }
    json!({"results": results})
}

fn carried_keys(params: &CallParams) -> Vec<&'static str> {
    let mut keys = Vec::new();
    if params.model.is_some() {
        keys.push("model");
    }
    if params.api_key.is_some() {
        keys.push("api_key");
    }
    if params.api_base.is_some() {
        keys.push("api_base");
    }
    if params.custom_llm_provider.is_some() {
        keys.push("custom_llm_provider");
    }
    if params.credentials.is_some() {
        keys.push(TRUSTED_CREDENTIALS_KEY);
    }
    keys.sort_unstable();
    keys
}

fn extra_body(fields: &ExtraBodyInput) -> crate::helpers::HelperResult {
    if let Some(body) = fields.body.as_ref() {
        check_extra_body(body).or_value_error()?;
    }
    Ok(json!({"body": fields.body}))
}
