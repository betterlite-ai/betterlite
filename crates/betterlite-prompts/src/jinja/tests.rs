#![cfg(test)]

use super::{LocalTemplate, extract_token_value, hf_chat_template};
use crate::builtin::{Calls, Content, Message};
use chrono::{TimeZone as _, Utc};
use indexmap::IndexMap;
use rstest::rstest;
use std::{fs, path::PathBuf, str};

#[rstest]
#[case::absent("null", "")]
#[case::plain_string(r#""tok""#, "tok")]
#[case::empty_string(r#""""#, "")]
#[case::object_with_content(r#"{"content":"c"}"#, "c")]
#[case::object_with_null_content(r#"{"content":null}"#, "")]
#[case::object_without_content("{}", "")]
fn jinja_prompts_behavior_tests_behavior(#[case] wire: &str, #[case] expected: &str) {
    let token = serde_json::from_str(wire).unwrap();
    assert_eq!(extract_token_value(token), expected);
}

#[rstest]
fn hf_chat_template_interpolates_supplied_tokens() {
    let template = LocalTemplate {
        chat_template: "{{ bos_token }}{{ eos_token }}".to_owned(),
        bos_token: Some(serde_json::from_str(r#""<s>""#).unwrap()),
        eos_token: Some(serde_json::from_str(r#"{"content":"</s>"}"#).unwrap()),
    };
    let messages = [Message {
        role: "user".to_owned(),
        content: Content::Text("hi".to_owned()),
        tool_calls: Calls::Absent,
        extra: IndexMap::new(),
    }];
    let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    assert_eq!(
        hf_chat_template(&template, &messages, now).unwrap(),
        "<s></s>"
    );
}

#[rstest]
fn replays_the_prompt_template_corpus(#[files("fuzz/corpus/prompt_template/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if bytes.len() > 0x0001_0000 {
        return;
    }
    let Ok(chat_template) = str::from_utf8(&bytes) else {
        return;
    };
    let template = LocalTemplate {
        chat_template: chat_template.to_owned(),
        bos_token: None,
        eos_token: None,
    };
    let messages = [Message {
        role: "user".to_owned(),
        content: Content::Text("hi".to_owned()),
        tool_calls: Calls::Absent,
        extra: IndexMap::new(),
    }];
    let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
    drop(hf_chat_template(&template, &messages, now));
}
