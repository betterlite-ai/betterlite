use rstest::rstest;

use crate::bedrock::projection::{project_message, project_messages, swaps_in_continue};
use crate::bedrock_tools::name::ToolNameMap;

use super::prelude::{json, list, messages};

#[rstest]
#[case::blank_assistant(r#"[{"role":"assistant","content":""}]"#, true)]
#[case::whitespace_assistant(r#"[{"role":"assistant","content":" \t\n"}]"#, true)]
#[case::information_separator(r#"[{"role":"assistant","content":"\u001c\u001f"}]"#, true)]
#[case::non_blank_assistant(r#"[{"role":"assistant","content":"x"}]"#, false)]
#[case::assistant_with_other_fields(
    r#"[{"role":"assistant","content":"","tool_calls":[]}]"#,
    false
)]
#[case::assistant_with_prefix(r#"[{"role":"assistant","content":"","prefix":false}]"#, false)]
#[case::user_blank(r#"[{"role":"user","content":""}]"#, false)]
#[case::assistant_without_content(r#"[{"role":"assistant"}]"#, false)]
#[case::assistant_with_list_content(
    r#"[{"role":"assistant","content":[{"type":"text","text":""}]}]"#,
    false
)]
fn bedrock_prompts_bedrock_prompt_tests_behavior_only_a_blank_assistant_string_swaps_in_the_continue_message(
    #[case] raw: &str,
    #[case] swaps: bool,
) {
    let message = messages(raw).into_iter().next().unwrap();
    assert_eq!(swaps_in_continue(&message), swaps);
}

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
    let (projected, _) = project_messages(
        &messages(
            r#"[{"role":"user","content":"hi","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get weather","arguments":"{}"}}
            ]}]"#,
        ),
        None,
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
    let (projected, _) = project_messages(
        &messages(
            r#"[{"role":"assistant","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get weather","arguments":"{}"}}
            ]}]"#,
        ),
        None,
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
    let (projected, _) = project_messages(
        &messages(
            r#"[{"role":"assistant","tool_calls":[
                {"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{}"}}
            ]}]"#,
        ),
        None,
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
    let (projected, _) = project_messages(&messages(raw), None, &mut names);
    assert_eq!(projected, list(raw));
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
    let (projected, _) = project_messages(&source, None, &mut names);
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
