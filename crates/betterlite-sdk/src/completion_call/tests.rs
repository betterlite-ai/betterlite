#![cfg(test)]

mod async_calls;
mod bridge_facts_answers;
mod coverage;
mod fallback_attempts;
mod probe_requests;
mod retry_policy;
mod support;
mod target_calls;
mod tokens_counting;

use betterlite_models::decisions::Json;

use crate::call_setup::correlation::CallOutcome;

const USER: &str = r#"{"role":"user","content":"hi"}"#;

fn body_text(outcome: &CallOutcome) -> Option<&Json> {
    match outcome {
        CallOutcome::Response(answer) => Some(answer),
        CallOutcome::ChatStream(_) | CallOutcome::ResponsesStream(_) => None,
    }
}

fn answer_choices(answer: &Json) -> &[Json] {
    let Json::Object(fields) = answer else {
        panic!("the answer is an object");
    };
    let Some(Json::List(choices)) = fields.get("choices") else {
        panic!("the answer carries choices");
    };
    choices
}

fn zero_retries(value: Option<&Json>) -> bool {
    matches!(value, Some(Json::Number(number)) if number.as_i64() == Some(0))
}
