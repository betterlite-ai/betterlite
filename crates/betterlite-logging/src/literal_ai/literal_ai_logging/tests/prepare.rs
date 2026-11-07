#![cfg(test)]

use betterlite_models::decisions::Json;
use chrono::Duration;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::prepare::{StepError, prepare_log_data};
use super::super::step::Step;
use super::support::{FixedIds, at, json, kwargs};

const START: &str = "2024-05-01T10:00:00.120Z";
const END: &str = "2024-05-01T10:00:01.500Z";

const CALLBACK: &str = r#"{
    "model": "gpt-4o",
    "custom_llm_provider": "openai",
    "litellm_params": {
        "metadata": {
            "step_id": "step-1",
            "literalai_thread_id": "thread-1",
            "literalai_parent_id": "parent-1",
            "literalai_root_run_id": "root-1",
            "tags": ["tag-a"],
            "literalai_tags": ["tag-b"]
        }
    },
    "standard_logging_object": {
        "metadata": {"user_api_key": "hashed"},
        "model_parameters": {"model": "gpt-4o", "temperature": 0.2, "tools": [{"type": "function"}]},
        "messages": [{"role": "user", "content": "hi"}],
        "response": {"choices": [{"message": {"role": "assistant", "content": "hello"}}]},
        "error_str": "upstream said no",
        "prompt_tokens": 3,
        "completion_tokens": 5,
        "total_tokens": 8
    }
}"#;

fn step(text: &str) -> Step {
    prepare_log_data(&kwargs(text), at(START), at(END), &FixedIds::default()).unwrap()
}

fn generation(step: &Step) -> IndexMap<String, Json> {
    let Json::Object(fields) = &step.to_fields()["generation"] else {
        panic!("the generation field is an object");
    };
    fields.clone()
}

#[test]
fn standard_logging_object_is_required() {
    let error = prepare_log_data(
        &kwargs(r#"{"model": "gpt-4o"}"#),
        at(START),
        at(END),
        &FixedIds::default(),
    )
    .unwrap_err();
    assert!(matches!(error, StepError::MissingStandardLoggingObject));
}

#[test]
fn the_step_carries_the_payload_and_the_callback_names() {
    let fields = step(CALLBACK).to_fields();
    assert_eq!(fields["id"], json(r#""step-1""#));
    assert_eq!(fields["name"], json(r#""gpt-4o""#));
    assert_eq!(fields["type"], json(r#""llm""#));
    assert_eq!(fields["threadId"], json(r#""thread-1""#));
    assert_eq!(fields["parentId"], json(r#""parent-1""#));
    assert_eq!(fields["rootRunId"], json(r#""root-1""#));
    assert_eq!(fields["tags"], json(r#"["tag-a"]"#));
    assert_eq!(fields["metadata"], json(r#"{"user_api_key": "hashed"}"#));
    assert_eq!(fields["error"], json(r#""upstream said no""#));
    assert_eq!(
        fields["startTime"],
        json(r#""2024-05-01T10:00:00.120000Z""#)
    );
    assert_eq!(fields["endTime"], json(r#""2024-05-01T10:00:01.500000Z""#));
    assert_eq!(fields["input"], Json::Null);
    assert_eq!(fields["output"], Json::Null);
}

#[test]
fn the_generation_carries_the_tokens_the_provider_and_the_completion() {
    let generation = generation(&step(CALLBACK));
    assert_eq!(generation["inputTokenCount"], json("3"));
    assert_eq!(generation["outputTokenCount"], json("5"));
    assert_eq!(generation["tokenCount"], json("8"));
    assert_eq!(generation["provider"], json(r#""openai""#));
    assert_eq!(generation["model"], json(r#""gpt-4o""#));
    assert_eq!(generation["duration"], json("1.38"));
    assert_eq!(
        generation["messageCompletion"],
        json(r#"{"role": "assistant", "content": "hello"}"#)
    );
    assert_eq!(
        generation["messages"],
        json(r#"[{"role": "user", "content": "hi"}]"#)
    );
    assert_eq!(generation["promptId"], Json::Null);
    assert_eq!(generation["variables"], Json::Null);
}

#[test]
fn tools_move_out_of_the_settings_and_into_the_generation() {
    let generation = generation(&step(CALLBACK));
    assert_eq!(generation["tools"], json(r#"[{"type": "function"}]"#));
    assert_eq!(
        generation["settings"],
        json(r#"{"model": "gpt-4o", "temperature": 0.2}"#)
    );
}

#[test]
fn a_literal_prompt_stays_on_its_message_and_fills_the_generation() {
    let text = r#"{
        "model": "gpt-4o",
        "standard_logging_object": {
            "messages": [{
                "role": "user",
                "content": "hi {{name}}",
                "__literal_prompt__": {"prompt_id": "prompt-1", "variables": {"name": "Ada"}, "uuid": "message-1"}
            }],
            "model_parameters": {},
            "response": {}
        }
    }"#;
    let prepared = step(text);
    let generation = generation(&prepared);
    assert_eq!(generation["promptId"], json(r#""prompt-1""#));
    assert_eq!(generation["variables"], json(r#"{"name": "Ada"}"#));
    assert_eq!(
        generation["messages"],
        json(
            r#"[{"role": "user", "content": "hi {{name}}", "__literal_prompt__": {"prompt_id": "prompt-1", "variables": {"name": "Ada"}, "uuid": "message-1"}, "uuid": "message-1", "templated": true}]"#
        )
    );
}

#[test]
fn the_last_literal_prompt_wins() {
    let text = r#"{
        "model": "gpt-4o",
        "standard_logging_object": {
            "messages": [
                {"role": "user", "content": "a", "__literal_prompt__": {"prompt_id": "first", "uuid": "m1"}},
                {"role": "user", "content": "b", "__literal_prompt__": {"prompt_id": "second", "variables": {"x": 1}, "uuid": "m2"}}
            ],
            "model_parameters": {},
            "response": {}
        }
    }"#;
    let generation = generation(&step(text));
    assert_eq!(generation["promptId"], json(r#""second""#));
    assert_eq!(generation["variables"], json(r#"{"x": 1}"#));
}

#[test]
fn literalai_tags_are_used_when_tags_are_absent() {
    let text = r#"{
        "model": "gpt-4o",
        "litellm_params": {"metadata": {"literalai_tags": ["tag-b"]}},
        "standard_logging_object": {"model_parameters": {}, "response": {}}
    }"#;
    assert_eq!(step(text).to_fields()["tags"], json(r#"["tag-b"]"#));
}

#[test]
fn a_null_tags_key_still_wins_over_literalai_tags() {
    let text = r#"{
        "model": "gpt-4o",
        "litellm_params": {"metadata": {"tags": null, "literalai_tags": ["tag-b"]}},
        "standard_logging_object": {"model_parameters": {}, "response": {}}
    }"#;
    assert_eq!(step(text).to_fields()["tags"], Json::Null);
}

#[rstest]
#[case::zero(0, "0.0")]
#[case::one_microsecond(1, "0.000001")]
#[case::one_and_a_half_seconds(1_500_000, "1.5")]
#[case::whole_day(86_400_000_000, "86400.0")]
#[case::negative(-1_500_000, "-1.5")]
fn the_duration_is_the_end_minus_the_start_in_seconds(#[case] offset: i64, #[case] expected: &str) {
    let start = at(START);
    let end = start
        .checked_add_signed(Duration::microseconds(offset))
        .unwrap();
    let prepared = prepare_log_data(&kwargs(CALLBACK), start, end, &FixedIds::default()).unwrap();
    assert_eq!(generation(&prepared)["duration"], json(expected));
}

#[test]
fn a_generated_id_is_used_when_the_metadata_has_no_step_id() {
    let text = r#"{
        "model": "gpt-4o",
        "standard_logging_object": {"model_parameters": {}, "response": {}}
    }"#;
    assert_eq!(step(text).to_fields()["id"], json(r#""generated-0""#));
}

#[test]
fn a_response_without_choices_leaves_the_completion_empty() {
    let text = r#"{
        "model": "gpt-4o",
        "standard_logging_object": {"model_parameters": {}, "response": {"choices": []}}
    }"#;
    assert_eq!(generation(&step(text))["messageCompletion"], Json::Null);
}

#[test]
fn messages_that_are_not_a_list_are_passed_through() {
    let text = r#"{
        "model": "gpt-4o",
        "standard_logging_object": {"model_parameters": {}, "response": {}, "messages": "hello"}
    }"#;
    let generation = generation(&step(text));
    assert_eq!(generation["messages"], json(r#""hello""#));
    assert_eq!(generation["promptId"], Json::Null);
}
