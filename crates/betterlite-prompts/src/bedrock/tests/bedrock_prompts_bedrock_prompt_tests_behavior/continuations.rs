use crate::bedrock::assembly::AssistantContinue;
use crate::bedrock::error::PromptError;
use crate::bedrock_processor::error::ProcessorError;

use super::prelude::{Harness, Settings, assistant_continue_with_call, list, messages};

const TOOL_THEN_USER: &str = r#"[
    {"role":"user","content":"a"},
    {"role":"tool","tool_call_id":"call_1","content":"result"},
    {"role":"user","content":"b"}
]"#;

const CONTINUE_OVERWRITE_TURNS: &str = r#"[
    {"role":"user","content":"a"},
    {"role":"assistant","content":""},
    {"role":"user","content":"b"},
    {"role":"assistant","content":"y","tool_calls":[
        {"id":"c2","type":"function","function":{"name":"get.weather","arguments":"{}"}}
    ]},
    {"role":"user","content":"c"}
]"#;

const CONTINUE_OVERWRITE_EXPECTED: &str = r#"[
    {"role":"user","content":[{"text":"a"}]},
    {"role":"assistant","content":[
        {"text":"x"},
        {"toolUse":{"input":{},"name":"get_weather","toolUseId":"c1"}}
    ]},
    {"role":"user","content":[{"text":"b"}]},
    {"role":"assistant","content":[
        {"text":"y"},
        {"toolUse":{"input":{},"name":"get_weather","toolUseId":"c2"}}
    ]},
    {"role":"user","content":[{"text":"c"}]}
]"#;

const CONTINUE_RESWAP_TURNS: &str = r#"[
    {"role":"user","content":"a"},
    {"role":"assistant","content":""},
    {"role":"user","content":"b"},
    {"role":"assistant","content":"y","tool_calls":[
        {"id":"c2","type":"function","function":{"name":"get.weather","arguments":"{}"}}
    ]},
    {"role":"user","content":"c"},
    {"role":"assistant","content":""},
    {"role":"user","content":"d"}
]"#;

const CONTINUE_RESWAP_EXPECTED: &str = r#"[
    {"role":"user","content":[{"text":"a"}]},
    {"role":"assistant","content":[
        {"text":"x"},
        {"toolUse":{"input":{},"name":"get_weather","toolUseId":"c1"}}
    ]},
    {"role":"user","content":[{"text":"b"}]},
    {"role":"assistant","content":[
        {"text":"y"},
        {"toolUse":{"input":{},"name":"get_weather","toolUseId":"c2"}}
    ]},
    {"role":"user","content":[{"text":"c"}]},
    {"role":"assistant","content":[
        {"text":"x"},
        {"toolUse":{"input":{},"name":"get_weather","toolUseId":"c1"}}
    ]},
    {"role":"user","content":[{"text":"d"}]}
]"#;

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_an_empty_user_text_takes_the_continue_message()
 {
    let harness = Harness::new();
    let user_continue = messages(r#"[{"role":"user","content":"say something"}]"#)
        .into_iter()
        .next()
        .unwrap();
    let settings = Settings::default()
        .with_modify_params()
        .with_user_continue(&user_continue);
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":""}]"#),
            &[],
            &settings,
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(r#"[{"role":"user","content":[{"text":"say something"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_an_empty_conversation_takes_the_continue_message()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(&[], &[], &Settings::default().with_modify_params())
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(r#"[{"role":"user","content":[{"text":"Please continue."}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_an_empty_conversation_without_the_flag_fails()
 {
    let harness = Harness::new();
    let error = harness
        .convert(&[], &[], &Settings::default())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptError::Processor(ProcessorError::NoMessages)
    ));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_consecutive_user_and_tool_turns_take_dummy_assistant_messages()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(TOOL_THEN_USER),
            &[],
            &Settings::default().with_modify_params(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"a"}]},
                {"role":"assistant","content":[{"text":"Please continue."}]},
                {"role":"user","content":[{"toolResult":{
                    "content":[{"text":"result"}],"toolUseId":"call_1"
                }}]},
                {"role":"assistant","content":[{"text":"Please continue."}]},
                {"role":"user","content":[{"text":"b"}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_custom_assistant_continue_string_is_used()
{
    let harness = Harness::new();
    let continue_message = AssistantContinue::Text("keep going".to_owned());
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let converted = harness
        .convert(&messages(TOOL_THEN_USER), &[], &settings)
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"a"}]},
                {"role":"assistant","content":[{"text":"keep going"}]},
                {"role":"user","content":[{"toolResult":{
                    "content":[{"text":"result"}],"toolUseId":"call_1"
                }}]},
                {"role":"assistant","content":[{"text":"keep going"}]},
                {"role":"user","content":[{"text":"b"}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_custom_assistant_continue_message_keeps_its_text()
 {
    let harness = Harness::new();
    let continue_message = AssistantContinue::Message(
        messages(r#"[{"role":"assistant","content":[{"type":"text","text":"go on"}]}]"#)
            .into_iter()
            .next()
            .unwrap(),
    );
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let converted = harness
        .convert(&messages(TOOL_THEN_USER), &[], &settings)
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"a"}]},
                {"role":"assistant","content":[{"text":"go on"}]},
                {"role":"user","content":[{"toolResult":{
                    "content":[{"text":"result"}],"toolUseId":"call_1"
                }}]},
                {"role":"assistant","content":[{"text":"go on"}]},
                {"role":"user","content":[{"text":"b"}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_continue_message_tool_call_is_made_valid_and_mapped()
 {
    let harness = Harness::new();
    let continue_message = assistant_continue_with_call();
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"a"},{"role":"assistant","content":""}]"#),
            &[],
            &settings,
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"a"}]},
                {"role":"assistant","content":[
                    {"text":"x"},
                    {"toolUse":{"input":{},"name":"get_weather","toolUseId":"c1"}}
                ]}]"#
        )
    );
    assert_eq!(
        converted.names.get("get_weather").map(String::as_str),
        Some("get weather")
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_an_unused_continue_message_tool_call_adds_no_mapping()
 {
    let harness = Harness::new();
    let continue_message = assistant_continue_with_call();
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"a"}]"#),
            &[],
            &settings,
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(r#"[{"role":"user","content":[{"text":"a"}]}]"#)
    );
    assert!(converted.names.is_empty());
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_continue_message_between_user_turns_adds_no_mapping()
 {
    let harness = Harness::new();
    let continue_message = assistant_continue_with_call();
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let converted = harness
        .convert(&messages(TOOL_THEN_USER), &[], &settings)
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"a"}]},
                {"role":"assistant","content":[{"text":"x"}]},
                {"role":"user","content":[{"toolResult":{
                    "content":[{"text":"result"}],"toolUseId":"call_1"
                }}]},
                {"role":"assistant","content":[{"text":"x"}]},
                {"role":"user","content":[{"text":"b"}]}]"#
        )
    );
    assert!(converted.names.is_empty());
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_later_history_call_overwrites_the_continue_mapping()
 {
    let harness = Harness::new();
    let continue_message = assistant_continue_with_call();
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let converted = harness
        .convert(&messages(CONTINUE_OVERWRITE_TURNS), &[], &settings)
        .await
        .unwrap();
    assert_eq!(converted.messages, list(CONTINUE_OVERWRITE_EXPECTED));
    assert_eq!(
        converted.names.get("get_weather").map(String::as_str),
        Some("get.weather")
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_second_blank_turn_records_the_continue_name_again()
 {
    let harness = Harness::new();
    let continue_message = assistant_continue_with_call();
    let settings = Settings::default().with_assistant_continue(&continue_message);
    let converted = harness
        .convert(&messages(CONTINUE_RESWAP_TURNS), &[], &settings)
        .await
        .unwrap();
    assert_eq!(converted.messages, list(CONTINUE_RESWAP_EXPECTED));
    assert_eq!(
        converted.names.get("get_weather").map(String::as_str),
        Some("get weather")
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_prefix_assistant_turn_takes_no_continue_message()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":"say"},
                    {"role":"assistant","content":"partial","prefix":true}]"#,
            ),
            &[],
            &Settings::default().with_modify_params(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"say"}]},
                {"role":"assistant","content":[{"text":"partial"}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_leading_assistant_turn_takes_a_continue_message()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"assistant","content":"first"},{"role":"user","content":"second"}]"#,
            ),
            &[],
            &Settings::default().with_modify_params(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"Please continue."}]},
                {"role":"assistant","content":[{"text":"first"}]},
                {"role":"user","content":[{"text":"second"}]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_blank_final_assistant_turn_takes_a_trailing_continue_message()
 {
    let harness = Harness::new();
    let converted = harness
        .convert(
            &messages(r#"[{"role":"user","content":"a"},{"role":"assistant","content":"   "}]"#),
            &[],
            &Settings::default().with_modify_params(),
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[{"text":"a"}]},
                {"role":"assistant","content":[{"text":"Please continue."}]},
                {"role":"user","content":[{"text":"Please continue."}]}]"#
        )
    );
}
