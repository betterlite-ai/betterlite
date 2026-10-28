#![cfg(test)]

use super::map_system_message_pt;
use crate::builtin::{Field, Message, PromptError};
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
#[case::system_map_metadata(r#"[{"role":"system","content":"s"},{"role":"assistant","content":"a","name":"named","tool_calls":[],"extra":{"enabled":true}}]"#, Some(r#"[{"role":"assistant","content":"s a","name":"named","tool_calls":[],"extra":{"enabled":true}}]"#))]
#[case::system_map_tool_presence(r#"[{"role":"assistant","content":"a","tool_calls":null},{"role":"assistant","content":"b","tool_calls":[]},{"role":"assistant","content":"c","tool_calls":[{"id":"id","type":"function","function":{"name":"f","arguments":"{}","strict":true},"index":1}]}]"#, Some(r#"[{"role":"assistant","content":"a","tool_calls":null},{"role":"assistant","content":"b","tool_calls":[]},{"role":"assistant","content":"c","tool_calls":[{"id":"id","type":"function","function":{"name":"f","arguments":"{}","strict":true},"index":1}]}]"#))]
fn builtin_prompts_behavior_tests_behavior(#[case] wire: &str, #[case] expected: Option<&str>) {
    let messages: Vec<Message> = serde_json::from_str(wire).unwrap();
    let actual = map_system_message_pt(&messages);
    match expected {
        Some(text) => assert_eq!(
            serde_json::to_value(actual.unwrap()).unwrap(),
            serde_json::to_value(serde_json::from_str::<Vec<Field>>(text).unwrap()).unwrap()
        ),
        None => assert!(matches!(actual, Err(PromptError::Text { .. }))),
    }
}
