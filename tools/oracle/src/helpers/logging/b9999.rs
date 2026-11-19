use betterlite_logging::callback_logger::event::GuardrailRequestSnapshot;
use betterlite_logging::guardrail_snapshot::{
    request_guardrail_fingerprint, request_mutating_guardrail_ran,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MutatingRan {
        metadata: Map<String, Value>,
    },
    FingerprintsMatch {
        first: Map<String, Value>,
        second: Map<String, Value>,
    },
    Capture {
        body: Map<String, Value>,
        metadata: Map<String, Value>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MutatingRan { metadata } => mutating_ran_answer(metadata),
            Input::FingerprintsMatch { first, second } => fingerprints_match_answer(first, second),
            Input::Capture { body, metadata } => capture_answer(body, metadata),
        }
    }
}

fn mutating_ran_answer(metadata: &Map<String, Value>) -> HelperResult {
    let fields: IndexMap<String, Json> = serde_json::from_value(Value::Object(metadata.clone()))?;
    Ok(json!({"result": request_mutating_guardrail_ran(&fields)}))
}

fn fingerprints_match_answer(
    first: &Map<String, Value>,
    second: &Map<String, Value>,
) -> HelperResult {
    let first_fields: IndexMap<String, Json> =
        serde_json::from_value(Value::Object(first.clone()))?;
    let second_fields: IndexMap<String, Json> =
        serde_json::from_value(Value::Object(second.clone()))?;
    let first_fingerprint = request_guardrail_fingerprint(&first_fields);
    let second_fingerprint = request_guardrail_fingerprint(&second_fields);
    Ok(json!({"result": first_fingerprint == second_fingerprint}))
}

fn capture_answer(body: &Map<String, Value>, metadata: &Map<String, Value>) -> HelperResult {
    let typed_body: IndexMap<String, Json> = serde_json::from_value(Value::Object(body.clone()))?;
    let typed_metadata: IndexMap<String, Json> =
        serde_json::from_value(Value::Object(metadata.clone()))?;
    let Some(snapshot) = GuardrailRequestSnapshot::capture(&typed_body, &typed_metadata) else {
        return Ok(json!({"result": null}));
    };
    let snapshot_body = serde_json::to_value(&snapshot.body)?;
    let current = request_guardrail_fingerprint(&typed_metadata);
    Ok(json!({
        "result": {
            "body": snapshot_body,
            "fingerprint_matches": current.as_deref() == Some(snapshot.fingerprint.as_str()),
        }
    }))
}
