use betterlite_core::api::request_guardrail_policy::GuardrailMetadata;
use betterlite_guardrails::passthrough_policy::{
    JsonPathValue, collect_guardrails, evaluate, extract_fields, get_settings, normalize_config,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Evaluate {
        data: Json,
        expression: String,
    },
    ExtractFields {
        data: Json,
        expressions: Vec<String>,
    },
    NormalizeConfig {
        config: Option<Json>,
    },
    GetSettings {
        config: Option<Json>,
        name: String,
    },
    CollectGuardrails {
        key_metadata: GuardrailMetadata,
        team_metadata: GuardrailMetadata,
        config: Option<Json>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers = input.calls.iter().map(answer).collect();
        Ok(Value::Array(answers))
    }
}

fn answer(call: &Call) -> Value {
    match call {
        Call::Evaluate { data, expression } => match evaluate(data, expression) {
            Some(JsonPathValue::Single(text)) => json!(text),
            Some(JsonPathValue::Many(texts)) => json!(texts),
            None => Value::Null,
        },
        Call::ExtractFields { data, expressions } => json!(extract_fields(data, expressions)),
        Call::NormalizeConfig { config } => {
            normalize_config(config.as_ref()).map_or(Value::Null, |parsed| json!(parsed))
        }
        Call::GetSettings { config, name } => {
            get_settings(config.as_ref(), name).map_or(Value::Null, |found| json!(found))
        }
        Call::CollectGuardrails {
            key_metadata,
            team_metadata,
            config,
        } => collect_guardrails(key_metadata, team_metadata, config.as_ref())
            .map_or(Value::Null, |collected| json!(collected)),
    }
}
