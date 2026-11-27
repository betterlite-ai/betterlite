use betterlite_core::api::summary_execution::extract::{
    CompletionAnswer, extract_response_text, extract_usage,
};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

// the raw completion objects _extract_response_text and _extract_usage read on the LiteLLM side
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    responses: Vec<Value>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut answers = Vec::with_capacity(input.responses.len());
        for raw in &input.responses {
            let answer: CompletionAnswer =
                serde_json::from_value(raw.clone()).map_err(|error| {
                    failure(format!("a completion object does not decode: {error}"))
                })?;
            answers.push(json!({
                "text": text_answer(&answer),
                "usage": usage_answer(&answer),
            }));
        }
        Ok(json!({ "answers": answers }))
    }
}

// a TypeError from the join of a non-string text part is the only failure the extractor reports
fn text_answer(answer: &CompletionAnswer) -> Value {
    match extract_response_text(answer) {
        Ok(text) => json!({ "ok": text, "error": Value::Null }),
        Err(_non_text) => json!({ "ok": Value::Null, "error": "TypeError" }),
    }
}

// int(value or 0) raises ValueError for a usage count that is not an integer
fn usage_answer(answer: &CompletionAnswer) -> Value {
    match extract_usage(answer) {
        Ok(usage) => json!({
            "ok": [usage.input_tokens, usage.output_tokens],
            "error": Value::Null,
        }),
        Err(_not_integer) => json!({ "ok": Value::Null, "error": "ValueError" }),
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SummaryExtractionError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
