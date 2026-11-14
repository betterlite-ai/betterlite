use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::unified_dispatch::transform_chunk::{
    TransformChunkInput, build_transform_chunk,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{
    error_json, int_index, int_pairs, reason_index, text_index, text_pairs, to_json,
};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub(super) reference_chunk: GuardrailValue,
    pub(super) mutated: Vec<(i64, String)>,
    pub(super) emitted: Vec<(i64, String)>,
    pub(super) holdback: Vec<(i64, i64)>,
    #[serde(default)]
    pub(super) reasons: Vec<(i64, Option<String>)>,
    #[serde(default)]
    pub(super) held: Vec<(i64, i64)>,
    pub(super) is_final: bool,
}

pub(super) fn run(request: &Request) -> HelperResult {
    let mutated = text_index(&request.mutated);
    let mut emitted = text_index(&request.emitted);
    let holdback = int_index(&request.holdback);
    let reasons = reason_index(&request.reasons);
    let mut held = int_index(&request.held);
    let outcome = build_transform_chunk(TransformChunkInput {
        reference_chunk: &request.reference_chunk,
        mutated_text_per_choice: &mutated,
        emitted_text_per_choice: &mut emitted,
        holdback_per_choice: &holdback,
        finish_reason_per_choice: &reasons,
        held_chars_per_choice: &mut held,
        is_final: request.is_final,
    });
    match outcome {
        Ok(produced) => {
            let chunk = match produced {
                Some(value) => to_json(&value)?,
                None => Value::Null,
            };
            Ok(json!({
                "chunk": chunk,
                "emitted": text_pairs(&emitted),
                "held": int_pairs(&held),
            }))
        }
        Err(error) => Ok(json!({"error": error_json(&error)?})),
    }
}
