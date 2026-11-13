#![cfg(test)]

use serde_json::{Value, json};

use super::{ReplyInput, TextTokenCounter, transform_response};

struct WordCounter;

impl TextTokenCounter for WordCounter {
    fn count_response_tokens(&self, text: &str) -> u64 {
        u64::try_from(text.split_whitespace().count()).unwrap_or(u64::MAX)
    }
}

fn input<'request>(reply: &'request Value, prompt: &'request str) -> ReplyInput<'request> {
    ReplyInput {
        reply,
        model: "endpoint-model",
        prompt,
        id: String::from("chatcmpl-default"),
        created: 1_700_000_000,
    }
}

fn content(reply: &Value, prompt: &str) -> String {
    let response = transform_response(&input(reply, prompt), &WordCounter).expect("a response");
    let choice = response.choices.first().expect("a choice");
    choice.message.content.clone().unwrap_or_default()
}

#[test]
fn generation_is_read_before_generated_text() {
    let reply = json!([{"generation": "from generation", "generated_text": "from text"}]);
    assert_eq!(content(&reply, ""), "from generation");
}

#[test]
fn generated_text_is_read_when_generation_is_absent() {
    let reply = json!({"generated_text": "from text"});
    assert_eq!(content(&reply, ""), "from text");
}

#[test]
fn a_reply_with_neither_field_is_an_empty_completion() {
    let reply = json!({"details": {}});
    assert_eq!(content(&reply, ""), "");
}

#[test]
fn the_first_list_item_is_read() {
    let reply = json!([{"generation": "first"}, {"generation": "second"}]);
    assert_eq!(content(&reply, ""), "first");
}

#[test]
fn an_echoed_prompt_is_stripped_when_the_template_has_a_bos_token() {
    let reply = json!({"generation": "<s>[INST] hi [/INST] there"});
    assert_eq!(content(&reply, "<s>[INST] hi [/INST]"), " there");
}

#[test]
fn a_prompt_without_a_bos_token_leaves_the_echo_in_place() {
    let reply = json!({"generation": "hello there"});
    assert_eq!(content(&reply, "hello"), "hello there");
}

#[test]
fn usage_counts_the_prompt_and_the_reply_text() {
    let reply = json!({"generation": "three four five"});
    let response = transform_response(&input(&reply, "one two"), &WordCounter).expect("a response");
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.completion_tokens, 3);
    assert_eq!(response.usage.total_tokens, 5);
}

#[test]
fn the_response_keeps_the_default_id_created_model_and_finish_reason() {
    let reply = json!({"generation": "hi"});
    let response = transform_response(&input(&reply, ""), &WordCounter).expect("a response");
    assert_eq!(response.id, "chatcmpl-default");
    assert_eq!(response.created, 1_700_000_000);
    assert_eq!(response.model, "endpoint-model");
    assert_eq!(response.object, "chat.completion");
    assert_eq!(
        response
            .choices
            .first()
            .and_then(|choice| choice.finish_reason.clone()),
        Some(String::from("stop"))
    );
}

#[test]
fn an_empty_list_fails_with_the_parse_error() {
    let reply = json!([]);
    let error = transform_response(&input(&reply, "hi"), &WordCounter).expect_err("a failure");
    assert_eq!(error.status_code, 500);
    assert_eq!(
        error.message,
        "LiteLLM Error: Unable to parse sagemaker RAW RESPONSE []"
    );
}

#[test]
fn a_non_string_generation_fails_with_the_parse_error() {
    let reply = json!([{"generation": null}]);
    let error = transform_response(&input(&reply, "hi"), &WordCounter).expect_err("a failure");
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains(r#"[{"generation":null}]"#));
}

#[test]
fn a_string_reply_fails_with_the_parse_error() {
    let reply = json!("just text");
    let error = transform_response(&input(&reply, "hi"), &WordCounter).expect_err("a failure");
    assert_eq!(error.status_code, 500);
    assert!(error.message.contains(r#""just text""#));
}

#[test]
fn a_null_reply_fails_with_the_parse_error() {
    let reply = Value::Null;
    let error = transform_response(&input(&reply, "hi"), &WordCounter).expect_err("a failure");
    assert_eq!(error.status_code, 500);
}
