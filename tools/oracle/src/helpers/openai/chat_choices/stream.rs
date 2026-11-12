use betterlite_openai::chat_choices::stream::{StreamError, chunk_parser};
use serde_json::{Value, to_value};
use uuid::Uuid;

use super::ChunkSteps;
use crate::helpers::{HelperError, HelperResult};

pub(super) fn run(steps: &ChunkSteps) -> HelperResult {
    let mut outputs = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let outcome = match chunk_parser(&step.chunk, Uuid::nil, || 7) {
            Ok(reply) => value(&[
                ("ok", Value::Bool(true)),
                ("chunk", to_value(reply).map_err(failure)?),
            ]),
            Err(error) => provider_outcome(&error),
        };
        outputs.push(outcome);
    }
    Ok(Value::Array(outputs))
}

fn provider_outcome(error: &StreamError) -> Value {
    match error {
        StreamError::Provider { status, .. } => {
            value(&[("ok", Value::Bool(false)), ("status", Value::from(*status))])
        }
        other => value(&[
            ("ok", Value::Bool(false)),
            ("error", Value::String(other.to_string())),
        ]),
    }
}

pub(super) fn value(fields: &[(&str, Value)]) -> Value {
    Value::Object(
        fields
            .iter()
            .map(|(name, field)| ((*name).to_owned(), field.clone()))
            .collect(),
    )
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: std::fmt::Display,
{
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
