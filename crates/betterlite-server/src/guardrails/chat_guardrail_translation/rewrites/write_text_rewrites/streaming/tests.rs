#![cfg(test)]

use super::apply_output_streaming;
use crate::guardrails::chat_guardrail_translation::extraction::apply_error::ApplyError;
use crate::guardrails::chat_guardrail_translation::rewrites::error::RewriteError;
use crate::guardrails::chat_guardrail_translation::rewrites::tests::support::chunks;

fn texts(responses: &[&str]) -> Vec<String> {
    responses.iter().map(|text| (*text).to_owned()).collect()
}

#[test]
fn write_text_rewrites_set_a_list_item_once_and_blank_it_afterwards() {
    let mut stream = chunks(
        r#"[
        {"choices": [{"index": 0, "delta": {"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}}]},
        {"choices": [{"index": 0, "delta": {"content": [{"type": "text", "text": "c"}, {"type": "text", "text": "d"}]}}]}]"#,
    );
    apply_output_streaming(&mut stream, &texts(&["X"]), &[(0, Some(1))]).unwrap();
    assert_eq!(
        stream,
        chunks(
            r#"[
        {"choices": [{"index": 0, "delta": {"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "X"}]}}]},
        {"choices": [{"index": 0, "delta": {"content": [{"type": "text", "text": "c"}, {"type": "text", "text": ""}]}}]}]"#
        )
    );
}

#[test]
fn write_text_rewrites_keep_the_last_text_given_for_a_key() {
    let mut stream = chunks(r#"[{"choices": [{"index": 0, "delta": {"content": "a"}}]}]"#);
    apply_output_streaming(&mut stream, &texts(&["X", "Y"]), &[(0, None), (0, None)]).unwrap();
    assert_eq!(
        stream,
        chunks(r#"[{"choices": [{"index": 0, "delta": {"content": "Y"}}]}]"#)
    );
}

#[test]
fn write_text_rewrites_read_a_message_when_the_choice_has_no_delta() {
    let mut stream = chunks(r#"[{"choices": [{"index": 0, "message": {"content": "a"}}]}]"#);
    apply_output_streaming(&mut stream, &texts(&["X"]), &[(0, None)]).unwrap();
    assert_eq!(
        stream,
        chunks(r#"[{"choices": [{"index": 0, "message": {"content": "X"}}]}]"#)
    );
}

#[test]
fn write_text_rewrites_treat_a_missing_index_as_choice_zero() {
    let mut stream = chunks(r#"[{"choices": [{"delta": {"content": "a"}}]}]"#);
    apply_output_streaming(&mut stream, &texts(&["X"]), &[(0, None)]).unwrap();
    assert_eq!(
        stream,
        chunks(r#"[{"choices": [{"delta": {"content": "X"}}]}]"#)
    );
}

#[test]
fn write_text_rewrites_leave_everything_else_alone() {
    let source = r#"[
        {"choices": [{"index": 1, "delta": {"content": "other"}}]},
        {"choices": [{"index": 0, "delta": {"content": null}}, {"index": 0, "delta": {"role": "assistant"}}]},
        {"choices": [{"index": 0, "delta": {"content": [{"type": "image_url"}, "junk"]}}]},
        {"choices": [{"index": 0}]},
        {"model": "m"},
        {"choices": null}]"#;
    let mut stream = chunks(source);
    apply_output_streaming(&mut stream, &texts(&["X"]), &[(0, None)]).unwrap();
    assert_eq!(stream, chunks(source));
}

#[test]
fn write_text_rewrites_refuse_more_texts_than_mappings() {
    let mut stream = chunks(r#"[{"choices": [{"index": 0, "delta": {"content": "a"}}]}]"#);
    let failure =
        apply_output_streaming(&mut stream, &texts(&["X", "Y"]), &[(0, None)]).unwrap_err();
    assert!(matches!(
        failure,
        RewriteError::Apply(ApplyError::UnmappedResponse { response_index: 1 })
    ));
}
