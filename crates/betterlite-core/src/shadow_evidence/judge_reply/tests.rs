#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::shape;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::valid_reply(
    r#"{"model":"judge-model","choices":[{"message":{"role":"assistant","content":"{\"preference\": \"A\", \"confidence\": 0.9}"},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=38 chars, model=judge-model"
)]
#[case::truncated_reply(
    r#"{"model":"judge-model","choices":[{"message":{"role":"assistant","content":"{\"preference\": \"A\""},"finish_reason":"length"}]}"#,
    "finish_reason=length, content=18 chars, model=judge-model"
)]
#[case::empty_content(
    r#"{"model":"judge-model","choices":[{"message":{"role":"assistant","content":""},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=no content, model=judge-model"
)]
#[case::null_content(
    r#"{"model":"judge-model","choices":[{"message":{"role":"assistant","content":null},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=no content, model=judge-model"
)]
#[case::absent_content(
    r#"{"model":"judge-model","choices":[{"message":{"role":"assistant"},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=no content, model=judge-model"
)]
#[case::zero_content_counts_as_no_content(
    r#"{"model":"judge-model","choices":[{"message":{"content":0},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=no content, model=judge-model"
)]
#[case::number_content_is_measured(
    r#"{"model":"judge-model","choices":[{"message":{"content":5},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=1 chars, model=judge-model"
)]
#[case::list_content_keeps_its_length(
    r#"{"model":"judge-model","choices":[{"message":{"content":["a"]},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=5 chars, model=judge-model"
)]
#[case::no_choices(r#"{"model":"judge-model","choices":[]}"#, "unreadable judge reply")]
#[case::null_message(
    r#"{"model":"judge-model","choices":[{"message":null,"finish_reason":"stop"}]}"#,
    "unreadable judge reply"
)]
#[case::absent_model(
    r#"{"choices":[{"message":{"content":"ab"}}]}"#,
    "finish_reason=unknown, content=2 chars, model=unknown"
)]
#[case::empty_model(
    r#"{"model":"","choices":[{"message":{"content":"ab"},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=2 chars, model=unknown"
)]
#[case::number_model_is_stringified(
    r#"{"model":5,"choices":[{"message":{"content":"ab"},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=2 chars, model=5"
)]
#[case::true_model_is_stringified(
    r#"{"model":true,"choices":[{"message":{"content":"ab"},"finish_reason":"stop"}]}"#,
    "finish_reason=stop, content=2 chars, model=True"
)]
#[case::finish_reason_comes_from_the_choice(
    r#"{"model":"m","choices":[{"message":{"content":"x"},"finish_reason":"tool_calls"}]}"#,
    "finish_reason=tool_calls, content=1 chars, model=m"
)]
fn shape_withholds_the_judge_text(#[case] response: &str, #[case] expected: &str) {
    assert_eq!(shape(&json(response)), expected);
}
