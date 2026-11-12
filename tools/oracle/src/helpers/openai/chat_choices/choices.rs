use betterlite_openai::chat_choices::decode_chat_choices;
use serde_json::{Value, to_value};
use uuid::Uuid;

use super::ChoiceSteps;
use crate::helpers::{HelperError, HelperResult};

pub(super) fn run(steps: &ChoiceSteps) -> HelperResult {
    let mut outputs = Vec::with_capacity(steps.steps.len());
    for step in &steps.steps {
        let choices_json = serde_json::to_string(&step.choices).map_err(failure)?;
        let reply = format!("{{\"choices\":{choices_json}}}");
        let choices = decode_chat_choices(
            reply.as_bytes(),
            step.optional_params.as_ref(),
            step.json_mode,
            nil_id,
        )
        .map_err(failure)?;
        outputs.push(to_value(choices).map_err(failure)?);
    }
    Ok(Value::Array(outputs))
}

fn nil_id() -> Uuid {
    Uuid::nil()
}

fn failure(error: impl std::fmt::Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ValueError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
