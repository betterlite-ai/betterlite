use betterlite_openai::chat_choices::decode_chat_choices;
use serde::Serialize;
use serde_json::{Value, to_value};
use uuid::Uuid;

use super::ChoiceSteps;
use crate::helpers::{HelperError, HelperResult};

#[derive(Serialize)]
struct Envelope<'step> {
    choices: &'step [Value],
}

pub(super) fn run(steps: &ChoiceSteps) -> HelperResult {
    let mut outputs = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let envelope = Envelope {
            choices: &step.choices,
        };
        let bytes = serde_json::to_vec(&envelope).map_err(failure)?;
        let choices = decode_chat_choices(
            &bytes,
            step.optional_params.as_ref(),
            step.json_mode,
            Uuid::nil,
        )
        .map_err(failure)?;
        outputs.push(to_value(choices).map_err(failure)?);
    }
    Ok(Value::Array(outputs))
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
