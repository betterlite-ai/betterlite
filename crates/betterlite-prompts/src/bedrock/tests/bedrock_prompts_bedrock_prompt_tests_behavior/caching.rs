use rstest::rstest;

use crate::arguments::json::Json;
use crate::bedrock_tools::capabilities::CachePointSupport;
use crate::message_content::Message;

use super::prelude::{Harness, RecordingCachePoints, Settings, json, list, messages};

const MODEL: &str = "anthropic.claude-sonnet-4-5-20250929-v1:0";

fn user_text_with_ttl(ttl: &str) -> Vec<Message> {
    messages(&format!(
        r#"[{{"role":"user","content":[{{"type":"text","text":"cache me",
            "cache_control":{{"type":"ephemeral","ttl":"{ttl}"}}}}]}}]"#
    ))
}

fn user_text_with_point(point: &str) -> Vec<Json> {
    list(&format!(
        r#"[{{"role":"user","content":[{{"text":"cache me"}},{{"cachePoint":{point}}}]}}]"#
    ))
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_content_cache_control_adds_a_point_after_the_block()
 {
    let harness = Harness::new();
    let recording = RecordingCachePoints::accepting();
    let settings = Settings::default().with_model(MODEL);
    let converted = harness
        .convert_with(
            &messages(
                r#"[{"role":"user","content":[
                    {"type":"text","text":"cache me","cache_control":{"type":"ephemeral"}}
                ]}]"#,
            ),
            &[],
            &settings,
            &recording,
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        user_text_with_point(r#"{"type":"default"}"#)
    );
    assert_eq!(
        recording.seen(),
        vec![(json(r#"{"type":"ephemeral"}"#), Some(MODEL.to_owned()))]
    );
}

#[rstest]
#[case::extended_ttl_kept(
    CachePointSupport::ExtendedTtl,
    Some(MODEL),
    "1h",
    r#"{"type":"default","ttl":"1h"}"#
)]
#[case::five_minute_ttl_kept(
    CachePointSupport::ExtendedTtl,
    Some(MODEL),
    "5m",
    r#"{"type":"default","ttl":"5m"}"#
)]
#[case::short_ttl_dropped(
    CachePointSupport::ShortTtl,
    Some(MODEL),
    "1h",
    r#"{"type":"default"}"#
)]
#[case::ttl_without_a_model_dropped(
    CachePointSupport::ExtendedTtl,
    None,
    "1h",
    r#"{"type":"default"}"#
)]
#[case::unknown_ttl_dropped(
    CachePointSupport::ExtendedTtl,
    Some(MODEL),
    "2h",
    r#"{"type":"default"}"#
)]
#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_the_injected_flag_decides_the_cache_point_ttl(
    #[case] support: CachePointSupport,
    #[case] model: Option<&str>,
    #[case] ttl: &str,
    #[case] expected: &str,
) {
    let harness = Harness::new();
    let mut settings = Settings::default().with_cache_points(support);
    if let Some(name) = model {
        settings = settings.with_model(name);
    }
    let converted = harness
        .convert(&user_text_with_ttl(ttl), &[], &settings)
        .await
        .unwrap();
    assert_eq!(converted.messages, user_text_with_point(expected));
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_an_unsupported_model_flag_adds_no_cache_point()
 {
    let harness = Harness::new();
    let settings = Settings::default().with_model(MODEL);
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"user","content":[
                    {"type":"text","text":"cache me","cache_control":{"type":"ephemeral"}}
                ]}]"#,
            ),
            &[],
            &settings,
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(r#"[{"role":"user","content":[{"text":"cache me"}]}]"#)
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_tool_message_cache_control_adds_a_cache_point()
 {
    let harness = Harness::new();
    let settings = Settings::default()
        .with_model(MODEL)
        .with_cache_points(CachePointSupport::ExtendedTtl);
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"tool","tool_call_id":"call_1","content":[
                    {"type":"text","text":"one","cache_control":{"type":"ephemeral","ttl":"1h"}}
                ]}]"#,
            ),
            &[],
            &settings,
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"user","content":[
                {"toolResult":{"content":[{"text":"one"}],"toolUseId":"call_1"}},
                {"cachePoint":{"type":"default","ttl":"1h"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_tool_call_cache_control_adds_a_cache_point()
 {
    let harness = Harness::new();
    let settings = Settings::default()
        .with_model(MODEL)
        .with_cache_points(CachePointSupport::ShortTtl);
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"assistant","content":null,"tool_calls":[
                    {"id":"call_1","type":"function","function":{"name":"f","arguments":"{}"},
                     "cache_control":{"type":"ephemeral","ttl":"1h"}}
                ]}]"#,
            ),
            &[],
            &settings,
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"assistant","content":[
                {"toolUse":{"input":{},"name":"f","toolUseId":"call_1"}},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}

#[tokio::test]
async fn bedrock_prompts_bedrock_prompt_tests_behavior_a_string_cache_control_adds_a_cache_point() {
    let harness = Harness::new();
    let settings = Settings::default()
        .with_model(MODEL)
        .with_cache_points(CachePointSupport::ShortTtl);
    let converted = harness
        .convert(
            &messages(
                r#"[{"role":"assistant","content":"answer","cache_control":{"type":"ephemeral"}}]"#,
            ),
            &[],
            &settings,
        )
        .await
        .unwrap();
    assert_eq!(
        converted.messages,
        list(
            r#"[{"role":"assistant","content":[
                {"text":"answer"},
                {"cachePoint":{"type":"default"}}
            ]}]"#
        )
    );
}
