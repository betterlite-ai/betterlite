use rstest::rstest;

use crate::bedrock::projection::{project_message, project_messages};
use crate::bedrock_tools::name::ToolNameMap;

use super::prelude::{json, list, messages};

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_a_text_message_projects_every_field() {
    let message = messages(
        r#"[{"role":"user","content":"hi","name":"bob","cache_control":{"type":"ephemeral"}}]"#,
    )
    .into_iter()
    .next()
    .unwrap();
    assert_eq!(
        project_message(&message),
        json(r#"{"role":"user","content":"hi","name":"bob","cache_control":{"type":"ephemeral"}}"#)
    );
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_a_message_without_content_projects_without_the_key()
 {
    let message = messages(r#"[{"role":"assistant","tool_calls":[]}]"#)
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        project_message(&message),
        json(r#"{"role":"assistant","tool_calls":[]}"#)
    );
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_content_parts_keep_their_type_and_fields() {
    let message = messages(
        r#"[{"role":"user","content":[
            {"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD"}},
            {"type":"text","text":"hi"}
        ]}]"#,
    )
    .into_iter()
    .next()
    .unwrap();
    assert_eq!(
        project_message(&message),
        json(
            r#"{"role":"user","content":[
                {"type":"image_url","image_url":{"url":"data:image/png;base64,QUJD"}},
                {"type":"text","text":"hi"}
            ]}"#
        )
    );
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_a_user_tool_call_name_is_not_made_valid() {
    let mut names = ToolNameMap::new();
    let projected = project_messages(
        &messages(
            r#"[{"role":"user","content":"hi","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get weather","arguments":"{}"}}
            ]}]"#,
        ),
        &mut names,
    );
    assert_eq!(
        projected,
        list(
            r#"[{"role":"user","content":"hi","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get weather","arguments":"{}"}}
            ]}]"#
        )
    );
    assert!(names.is_empty());
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_an_assistant_tool_call_name_is_made_valid_and_mapped()
 {
    let mut names = ToolNameMap::new();
    let projected = project_messages(
        &messages(
            r#"[{"role":"assistant","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get weather","arguments":"{}"}}
            ]}]"#,
        ),
        &mut names,
    );
    assert_eq!(
        projected,
        list(
            r#"[{"role":"assistant","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}
            ]}]"#
        )
    );
    assert_eq!(
        names.get("get_weather").map(String::as_str),
        Some("get weather")
    );
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_an_already_valid_tool_call_name_adds_no_mapping() {
    let mut names = ToolNameMap::new();
    let projected = project_messages(
        &messages(
            r#"[{"role":"assistant","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}
            ]}]"#,
        ),
        &mut names,
    );
    assert_eq!(
        projected,
        list(
            r#"[{"role":"assistant","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}
            ]}]"#
        )
    );
    assert!(names.is_empty());
}

#[rstest]
#[case::tool_calls_not_a_list(r#"[{"role":"assistant","tool_calls":{}}]"#)]
#[case::call_not_an_object(r#"[{"role":"assistant","tool_calls":[5]}]"#)]
#[case::function_not_an_object(r#"[{"role":"assistant","tool_calls":[{"function":"x"}]}]"#)]
#[case::name_not_text(r#"[{"role":"assistant","tool_calls":[{"function":{"name":5}}]}]"#)]
fn bedrock_prompts_bedrock_prompt_tests_behavior_tool_call_shapes_without_a_name_are_left_alone(
    #[case] raw: &str,
) {
    let mut names = ToolNameMap::new();
    assert_eq!(project_messages(&messages(raw), &mut names), list(raw));
    assert!(names.is_empty());
}

#[test]
fn bedrock_prompts_bedrock_prompt_tests_behavior_projection_leaves_the_source_messages_unchanged() {
    let source = messages(
        r#"[{"role":"user","content":"hi"},
            {"role":"assistant","content":null,"tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get weather","arguments":"{}"}}
            ]}]"#,
    );
    let mut names = ToolNameMap::new();
    let projected = project_messages(&source, &mut names);
    assert_eq!(names.len(), 1);
    assert_eq!(
        source,
        messages(
            r#"[{"role":"user","content":"hi"},
                {"role":"assistant","content":null,"tool_calls":[
                    {"id":"call_1","type":"function","function":{"name":"get weather","arguments":"{}"}}
                ]}]"#
        )
    );
    assert_eq!(
        projected,
        list(
            r#"[{"role":"user","content":"hi"},
                {"role":"assistant","tool_calls":[
                    {"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}
                ]}]"#
        )
    );
}
