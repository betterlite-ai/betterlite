#![cfg(test)]

use super::estimate::{InputOutputTokens, input_and_output_tokens};
use super::support::{FixedTranslation, chat_reply, estimate, no_translation, request};

fn output_of(body: &str, call_type: Option<&str>, configured: Option<i64>) -> i64 {
    let data = request(body);
    input_and_output_tokens(
        &no_translation(),
        &estimate(&data, call_type, Some(400), configured),
    )
    .output_tokens
}

#[test]
fn candidates_multiply_the_explicit_cap() {
    assert_eq!(
        output_of(
            r#"{"messages":[{"role":"user","content":"a"}],"max_tokens":10,"n":3}"#,
            Some("completion"),
            None
        ),
        30
    );
}

#[test]
fn candidates_multiply_a_zero_cap_into_zero() {
    assert_eq!(
        output_of(
            r#"{"messages":[{"role":"user","content":"a"}],"max_tokens":0,"n":3}"#,
            Some("completion"),
            None
        ),
        0
    );
}

#[test]
fn candidates_multiply_the_configured_estimate() {
    assert_eq!(
        output_of(
            r#"{"messages":[{"role":"user","content":"a"}],"best_of":2}"#,
            Some("completion"),
            Some(5)
        ),
        10
    );
}

#[test]
fn candidates_multiply_the_output_floor() {
    assert_eq!(
        output_of(
            r#"{"messages":[{"role":"user","content":"a"}],"n":2}"#,
            Some("completion"),
            None
        ),
        200
    );
}

#[test]
fn candidates_do_not_multiply_the_input_tokens() {
    let data =
        request(r#"{"messages":[{"role":"user","content":"abcdefgh"}],"max_tokens":1,"n":3}"#);
    assert_eq!(
        input_and_output_tokens(
            &no_translation(),
            &estimate(&data, Some("completion"), None, None)
        ),
        InputOutputTokens {
            input_tokens: 2,
            output_tokens: 3
        }
    );
}

#[test]
fn a_google_candidate_count_multiplies_the_output() {
    let data = request(r#"{"contents":[],"config":{"maxOutputTokens":10,"candidateCount":2}}"#);
    let translation = FixedTranslation {
        reply: Some(chat_reply(r#"[{"role":"user","content":"abcd"}]"#)),
    };
    assert_eq!(
        input_and_output_tokens(
            &translation,
            &estimate(&data, Some("generate_content"), None, None)
        ),
        InputOutputTokens {
            input_tokens: 1,
            output_tokens: 20
        }
    );
}
