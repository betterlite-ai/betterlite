#![cfg(test)]

use super::map_system_message_pt;
use crate::builtin::{Message, PromptError};
use rstest::rstest;

#[rstest]
#[case::system_map(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#, Some(r#"[{"role":"user","content":"s u"},{"role":"assistant","content":"a"}]"#))]
#[case::system_map_consecutive(r#"[{"role":"system","content":"s"},{"role":"system","content":"t"},{"role":"assistant","content":"a"},{"role":"system","content":"last"}]"#, Some(r#"[{"role":"user","content":"s"},{"role":"assistant","content":"t a"},{"role":"user","content":"last"}]"#))]
#[case::system_map_drop(
    r#"[{"role":"system","content":"s"},{"role":"tool","content":"t"}]"#,
    Some(r#"[{"role":"tool","content":"t"}]"#)
)]
#[case::system_map_parts(
    r#"[{"role":"system","content":[]},{"role":"user","content":"u"}]"#,
    None
)]
fn builtin_prompts_behavior_tests_behavior(#[case] wire: &str, #[case] expected: Option<&str>) {
    let messages: Vec<Message> = serde_json::from_str(wire).unwrap();
    let actual = map_system_message_pt(&messages);
    match expected {
        Some(text) => assert_eq!(
            actual.unwrap(),
            serde_json::from_str::<Vec<Message>>(text).unwrap()
        ),
        None => assert!(matches!(actual, Err(PromptError::Text { .. }))),
    }
}
