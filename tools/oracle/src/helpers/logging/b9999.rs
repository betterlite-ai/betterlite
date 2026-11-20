use betterlite_logging::callback_logger::event::GuardrailRequestSnapshot;
use betterlite_logging::guardrail_snapshot::{
    request_guardrail_fingerprint, request_mutating_guardrail_ran,
};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    MutatingRan {
        metadata: IndexMap<String, Json>,
    },
    FingerprintsMatch {
        first: IndexMap<String, Json>,
        second: IndexMap<String, Json>,
    },
    Capture {
        body: IndexMap<String, Json>,
        metadata: IndexMap<String, Json>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MutatingRan { metadata } => {
                Ok(json!({"result": request_mutating_guardrail_ran(metadata)}))
            }
            Input::FingerprintsMatch { first, second } => Ok(json!({
                "result": request_guardrail_fingerprint(first)
                    == request_guardrail_fingerprint(second),
            })),
            Input::Capture { body, metadata } => capture_answer(body, metadata),
        }
    }
}

fn capture_answer(
    body: &IndexMap<String, Json>,
    metadata: &IndexMap<String, Json>,
) -> HelperResult {
    let Some(snapshot) = GuardrailRequestSnapshot::capture(body, metadata) else {
        return Ok(json!({"result": null}));
    };
    let snapshot_body = serde_json::to_value(&snapshot.body)?;
    let current = request_guardrail_fingerprint(metadata);
    Ok(json!({
        "result": {
            "body": snapshot_body,
            "fingerprint_matches": current.as_deref() == Some(snapshot.fingerprint.as_str()),
        }
    }))
}
