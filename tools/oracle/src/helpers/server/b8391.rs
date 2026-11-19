use std::fmt::Display;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::guardrails::pipeline_slots::model_guardrails::merge_guardrails_with_existing;
use betterlite_server::guardrails::pipeline_slots::slots::{
    merge_pipeline_metadata_writes, policy_state_metadata,
};
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    PolicyState {
        data: Map<String, Value>,
    },
    MergeMetadataWrites {
        data: Map<String, Value>,
        modified: Map<String, Value>,
    },
    MergeGuardrails {
        data: Map<String, Value>,
        model_level_guardrails: Value,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::PolicyState { data } => Ok(match policy_state_metadata(data) {
                Some(bucket) => Value::Object(bucket.clone()),
                None => Value::Null,
            }),
            Input::MergeMetadataWrites { data, modified } => {
                let mut merged = data.clone();
                merge_pipeline_metadata_writes(&mut merged, modified);
                Ok(Value::Object(merged))
            }
            Input::MergeGuardrails {
                data,
                model_level_guardrails,
            } => {
                let merged = merge_guardrails_with_existing(data, model_level_guardrails)
                    .map_err(failure)?;
                let mut merged = Value::Object(merged);
                sort_guardrails(&mut merged);
                Ok(merged)
            }
        }
    }
}

fn sort_guardrails(merged: &mut Value) {
    let Some(metadata) = merged.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(Value::Array(guardrails)) = metadata.get_mut("guardrails") else {
        return;
    };
    guardrails.sort_by_key(Value::to_string);
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "ModelGuardrailsError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
