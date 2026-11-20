#![cfg(test)]

use super::support::{blocks, messages, model_response};
use crate::bedrock_checks::messages::{build, chunk_texts};
use crate::bedrock_message_selection::wire::Message;
use crate::bedrock_records::{Role, Source};

fn numbered_content(count: usize) -> String {
    let blocks: Vec<String> = (0..count)
        .map(|index| format!(r#"{{"type":"text","text":"block-{index}"}}"#))
        .collect();
    format!(r#"[{{"role":"user","content":[{}]}}]"#, blocks.join(","))
}

#[test]
fn input_messages_retag_every_role_as_user() {
    let wire = messages(
        r#"[{"role":"system","content":"rules"},{"role":"assistant","content":"earlier"},{"role":"user","content":"hi"}]"#,
    );
    let checks = build(Source::Input, Some(&wire), None);
    assert_eq!(checks.len(), 3);
    assert!(
        checks
            .iter()
            .all(|message| message.role == Some(Role::User))
    );
    assert_eq!(blocks(&checks[0]), vec!["rules".to_owned()]);
    assert_eq!(blocks(&checks[1]), vec!["earlier".to_owned()]);
    assert_eq!(blocks(&checks[2]), vec!["hi".to_owned()]);
}

#[test]
fn input_keeps_every_text_block_of_a_content_list() {
    let wire = messages(
        r#"[{"role":"user","content":[{"type":"text","text":"first"},"second",{"type":"image","source":{"data":"x"}}]}]"#,
    );
    let checks = build(Source::Input, Some(&wire), None);
    assert_eq!(checks.len(), 1);
    assert_eq!(
        blocks(&checks[0]),
        vec!["first".to_owned(), "second".to_owned()]
    );
}

#[test]
fn empty_text_blocks_are_not_sent() {
    let wire = messages(r#"[{"role":"user","content":[{"type":"text","text":""},""]}]"#);
    let checks = build(Source::Input, Some(&wire), None);
    assert!(checks.is_empty());
}

#[test]
fn input_without_messages_builds_nothing() {
    assert!(build(Source::Input, None, None).is_empty());
    let empty: Vec<Message> = Vec::new();
    assert!(build(Source::Input, Some(&empty), None).is_empty());
}

#[test]
fn ten_content_blocks_stay_in_one_message() {
    let texts: Vec<String> = (0_usize..10_usize)
        .map(|index| format!("text-{index}"))
        .collect();
    let chunked = chunk_texts(Role::User, &texts);
    assert_eq!(chunked.len(), 1);
    assert_eq!(blocks(&chunked[0]).len(), 10);
}

#[test]
fn messages_split_after_ten_content_blocks_without_truncation() {
    let wire = messages(&numbered_content(23));
    let checks = build(Source::Input, Some(&wire), None);
    assert_eq!(checks.len(), 3);
    assert_eq!(blocks(&checks[0]).len(), 10);
    assert_eq!(blocks(&checks[1]).len(), 10);
    assert_eq!(blocks(&checks[2]).len(), 3);
    assert_eq!(blocks(&checks[0])[0], "block-0");
    assert_eq!(blocks(&checks[0])[9], "block-9");
    assert_eq!(blocks(&checks[1])[0], "block-10");
    assert_eq!(blocks(&checks[2])[2], "block-22");
    assert!(
        checks
            .iter()
            .all(|message| message.role == Some(Role::User))
    );
}

#[test]
fn output_response_uses_the_assistant_role() {
    let response = model_response(
        r#"{"choices":[{"message":{"role":"assistant","content":"model says hi"}}]}"#,
    );
    let checks = build(Source::Output, None, Some(&response));
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].role, Some(Role::Assistant));
    assert_eq!(blocks(&checks[0]), vec!["model says hi".to_owned()]);
}

#[test]
fn output_without_choices_builds_nothing() {
    let response = model_response(r#"{"choices":[]}"#);
    assert!(build(Source::Output, Some(&[]), Some(&response)).is_empty());
}

#[test]
fn output_empty_text_is_not_sent() {
    let response = model_response(r#"{"choices":[{"message":{"role":"assistant","content":""}}]}"#);
    assert!(build(Source::Output, None, Some(&response)).is_empty());
}
