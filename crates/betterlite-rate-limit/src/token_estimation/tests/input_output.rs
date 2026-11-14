#![cfg(test)]

use super::estimate::{InputOutputTokens, input_and_output_tokens};
use super::support::{FixedTranslation, chat_reply, estimate, no_translation, request};

fn tokens(body: &str, call_type: Option<&str>, min_limit: Option<i64>) -> InputOutputTokens {
    let data = request(body);
    input_and_output_tokens(
        &no_translation(),
        &estimate(&data, call_type, min_limit, None),
    )
}

#[test]
fn chat_messages_supply_the_input_text() {
    assert_eq!(
        tokens(
            r#"{"messages":[{"role":"user","content":"abcdefgh"}],"max_tokens":10}"#,
            Some("completion"),
            None
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 10
        }
    );
}

#[test]
fn a_text_completion_reads_the_prompt_and_ignores_the_messages() {
    assert_eq!(
        tokens(
            r#"{"messages":[{"role":"user","content":"abcdefghijkl"}],"prompt":"abcdefgh"}"#,
            Some("text_completion"),
            Some(400)
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 100
        }
    );
}

#[test]
fn a_completion_reads_the_messages_and_ignores_the_prompt() {
    assert_eq!(
        tokens(
            r#"{"messages":[{"role":"user","content":"abcdefgh"}],"prompt":"abcdefghijkl"}"#,
            Some("completion"),
            Some(400)
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 100
        }
    );
}

#[test]
fn a_responses_request_reads_the_input() {
    assert_eq!(
        tokens(
            r#"{"input":"abcdefgh","messages":[{"role":"user","content":"abcdefghijkl"}]}"#,
            Some("responses"),
            Some(400)
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 100
        }
    );
}

#[test]
fn an_embedding_request_reads_the_input_and_budgets_no_output() {
    assert_eq!(
        tokens(r#"{"input":"abcdefgh"}"#, Some("embedding"), Some(400)),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 0
        }
    );
}

#[test]
fn without_a_call_type_messages_win_over_the_prompt() {
    assert_eq!(
        tokens(
            r#"{"messages":[{"role":"user","content":"abcdefgh"}],"prompt":"abcdefghijkl"}"#,
            None,
            Some(400)
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 100
        }
    );
}

#[test]
fn without_a_call_type_an_input_field_supplies_the_input_text() {
    assert_eq!(
        tokens(r#"{"input":"abcdefgh"}"#, None, None),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 0
        }
    );
}

#[test]
fn an_empty_message_list_falls_back_to_the_prompt() {
    assert_eq!(
        tokens(r#"{"messages":[],"prompt":"abcdefgh"}"#, None, Some(400)),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 100
        }
    );
}

#[test]
fn an_empty_request_estimates_nothing() {
    assert_eq!(
        tokens("{}", None, None),
        InputOutputTokens {
            input_tokens: 0,
            output_tokens: 0
        }
    );
}

#[test]
fn a_named_call_type_without_matching_fields_estimates_nothing() {
    assert_eq!(
        tokens(
            r#"{"prompt":null,"input":"abcdefgh"}"#,
            Some("completion"),
            Some(400)
        ),
        InputOutputTokens {
            input_tokens: 0,
            output_tokens: 0
        }
    );
}

#[test]
fn a_prompt_list_counts_every_item() {
    assert_eq!(
        tokens(
            r#"{"prompt":["abcd","efgh"]}"#,
            Some("text_completion"),
            Some(400)
        )
        .input_tokens,
        2
    );
}

#[test]
fn an_input_list_counts_every_item() {
    assert_eq!(
        tokens(r#"{"input":["abcd","efgh"]}"#, Some("embedding"), None).input_tokens,
        2
    );
}

#[test]
fn the_output_floor_applies_when_no_cap_is_configured() {
    assert_eq!(
        tokens(
            r#"{"messages":[{"role":"user","content":"abcdefgh"}]}"#,
            Some("completion"),
            Some(400)
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 100
        }
    );
}

#[test]
fn a_configured_output_estimate_replaces_the_floor() {
    let data = request(r#"{"messages":[{"role":"user","content":"abcdefgh"}]}"#);
    assert_eq!(
        input_and_output_tokens(
            &no_translation(),
            &estimate(&data, Some("completion"), Some(400), Some(50))
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 50
        }
    );
}

#[test]
fn an_explicit_cap_wins_over_the_configured_estimate() {
    let data = request(r#"{"messages":[{"role":"user","content":"abcdefgh"}],"max_tokens":20}"#);
    assert_eq!(
        input_and_output_tokens(
            &no_translation(),
            &estimate(&data, Some("completion"), Some(400), Some(50))
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 20
        }
    );
}

#[test]
fn a_responses_floor_never_goes_below_sixteen() {
    assert_eq!(
        tokens(r#"{"input":"a"}"#, Some("responses"), Some(8)),
        InputOutputTokens {
            input_tokens: 1,
            output_tokens: 16
        }
    );
}

#[test]
fn without_a_configured_limit_the_floor_is_the_baseline() {
    assert_eq!(
        tokens(r#"{"input":"a"}"#, Some("responses"), None),
        InputOutputTokens {
            input_tokens: 1,
            output_tokens: 1024
        }
    );
}

#[test]
fn translated_google_messages_supply_the_input_text() {
    let data = request(r#"{"contents":[{"parts":[{"text":"ignored"}]}]}"#);
    let translation = FixedTranslation {
        reply: Some(chat_reply(r#"[{"role":"user","content":"abcdefgh"}]"#)),
    };
    assert_eq!(
        input_and_output_tokens(
            &translation,
            &estimate(&data, Some("generate_content"), Some(400), None)
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 100
        }
    );
}
