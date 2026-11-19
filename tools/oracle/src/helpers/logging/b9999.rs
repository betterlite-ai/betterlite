use betterlite_logging::callback_logger::event::GuardrailRequestSnapshot;
use betterlite_logging::guardrail_snapshot::{
    request_guardrail_fingerprint,
    // ast-grep-ignore: no-long-names
    request_mutating_guardrail_ran,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use betterlite_oracle_helper_api::error::HelperError;
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
    let fields = typed_fields(metadata)?;
    // ast-grep-ignore: no-long-names
    Ok(json!({"result": request_mutating_guardrail_ran(&fields)}))
}

fn fingerprints_match_answer(
    first: &Map<String, Value>,
    second: &Map<String, Value>,
) -> HelperResult {
    let first_fingerprint = request_guardrail_fingerprint(&typed_fields(first)?);
    let second_fingerprint = request_guardrail_fingerprint(&typed_fields(second)?);
    Ok(json!({"result": first_fingerprint == second_fingerprint}))
}

fn capture_answer(body: &Map<String, Value>, metadata: &Map<String, Value>) -> HelperResult {
    let typed_body = typed_fields(body)?;
    let typed_metadata = typed_fields(metadata)?;
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

fn typed_fields(fields: &Map<String, Value>) -> Result<IndexMap<String, Json>, Box<HelperError>> {
    Ok(serde_json::from_value(Value::Object(fields.clone()))?)
}
