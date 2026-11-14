use betterlite_prompts::anthropic_results::tool_result::ToolResultError;
use betterlite_prompts::arguments::json::Json;

use crate::snowflake::tests::support::{MediaSource, messages, parsed};

use super::files::FileBlockError;
use super::tools::{ToolError, transform_tool_choice, transform_tools};
use super::{MessageError, SplitMessages, extract_system_and_messages};

async fn split_turns(source: &str) -> SplitMessages {
    extract_system_and_messages(&messages(source), &MediaSource::default())
        .await
        .unwrap()
}

fn tool_list(source: &str) -> Vec<Json> {
    serde_json::from_str(source).unwrap()
}

fn conversation(split: &SplitMessages) -> Json {
    Json::List(split.conversation.clone())
}

fn system(split: &SplitMessages) -> Json {
    Json::List(split.system.clone().unwrap_or_default())
}

#[tokio::test]
async fn a_non_empty_system_string_becomes_one_text_block() {
    let split =
        split_turns(r#"[{"role":"system","content":"be nice"},{"role":"user","content":"hi"}]"#)
            .await;
    assert_eq!(
        system(&split),
        parsed(r#"[{"type":"text","text":"be nice"}]"#)
    );
    assert_eq!(
        conversation(&split),
        parsed(r#"[{"role":"user","content":"hi"}]"#)
    );
}

#[tokio::test]
async fn an_empty_or_absent_system_adds_nothing() {
    for source in [
        r#"[{"role":"system","content":""},{"role":"user","content":"hi"}]"#,
        r#"[{"role":"user","content":"hi"}]"#,
    ] {
        let split = split_turns(source).await;
        assert_eq!(split.system, None);
    }
}

#[tokio::test]
async fn a_system_list_keeps_only_text_blocks_with_their_cache_control() {
    let split = split_turns(
        r#"[{"role":"system","content":[{"type":"text","text":"one"},{"type":"text"},{"type":"image_url","image_url":"data:image/png;base64,AAA"},{"type":"text","text":"two","cache_control":{"type":"ephemeral","ttl":"5m"}}]}]"#,
    )
    .await;
    assert_eq!(
        system(&split),
        parsed(
            r#"[{"type":"text","text":"one"},{"type":"text","text":""},{"type":"text","text":"two","cache_control":{"type":"ephemeral","ttl":"5m"}}]"#
        )
    );
    assert!(split.conversation.is_empty());
}

#[tokio::test]
async fn an_assistant_tool_turn_puts_thinking_then_text_then_tool_use() {
    let split = split_turns(
        r#"[{"role":"assistant","content":"sure","tool_calls":[{"id":"call_1","type":"function","function":{"name":"f","arguments":"{\"a\":1}"}}],"thinking_blocks":[{"type":"thinking","thinking":"t","signature":"sig"},{"type":"thinking","thinking":"unsigned"},{"type":"redacted_thinking","data":"x"}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"sig"},{"type":"redacted_thinking","data":"x"},{"type":"text","text":"sure"},{"type":"tool_use","id":"call_1","name":"f","input":{"a":1}}]}]"#
        )
    );
}

#[tokio::test]
async fn tool_arguments_decode_or_fall_back() {
    let split = split_turns(
        r#"[{"role":"assistant","content":"","tool_calls":[{"id":"a","function":{"name":"f","arguments":"not json"}},{"id":"b","function":{"name":"g","arguments":5}},{"id":"c","function":{"name":"h"}}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"assistant","content":[{"type":"tool_use","id":"a","name":"f","input":{}},{"type":"tool_use","id":"b","name":"g","input":5},{"type":"tool_use","id":"c","name":"h","input":{}}]}]"#
        )
    );
}

#[tokio::test]
async fn an_assistant_tool_turn_wraps_list_content_in_one_text_block() {
    let split = split_turns(
        r#"[{"role":"assistant","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}],"tool_calls":[{"id":"x","function":{"name":"f","arguments":"{}"}}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"assistant","content":[{"type":"text","text":[{"type":"text","text":"a"},{"type":"text","text":"b"}]},{"type":"tool_use","id":"x","name":"f","input":{}}]}]"#
        )
    );
}

#[tokio::test]
async fn a_thinking_only_assistant_keeps_list_content_after_the_blocks() {
    let split = split_turns(
        r#"[{"role":"assistant","content":[{"type":"text","text":"answer"}],"thinking_blocks":[{"type":"thinking","thinking":"t","signature":"sig"}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"sig"},{"type":"text","text":"answer"}]}]"#
        )
    );
}

#[tokio::test]
async fn a_thinking_only_assistant_turns_string_content_into_one_text_block() {
    let split = split_turns(
        r#"[{"role":"assistant","content":"answer","thinking_blocks":[{"type":"thinking","thinking":"t","signature":"sig"}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"sig"},{"type":"text","text":"answer"}]}]"#
        )
    );
    let empty = split_turns(
        r#"[{"role":"assistant","content":"","thinking_blocks":[{"type":"thinking","thinking":"t","signature":"sig"}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&empty),
        parsed(
            r#"[{"role":"assistant","content":[{"type":"thinking","thinking":"t","signature":"sig"}]}]"#
        )
    );
}

#[tokio::test]
async fn an_assistant_without_thinking_or_calls_keeps_its_content() {
    let split = split_turns(r#"[{"role":"assistant","content":"plain"}]"#).await;
    assert_eq!(
        conversation(&split),
        parsed(r#"[{"role":"assistant","content":"plain"}]"#)
    );
    let listed =
        split_turns(r#"[{"role":"assistant","content":[{"type":"text","text":"a"}]}]"#).await;
    assert_eq!(
        conversation(&listed),
        parsed(r#"[{"role":"assistant","content":[{"type":"text","text":"a"}]}]"#)
    );
}

#[tokio::test]
async fn a_plain_tool_message_becomes_a_tool_result() {
    let split = split_turns(
        r#"[{"role":"tool","tool_call_id":"call_1","content":"result","cache_control":{"type":"ephemeral"}}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":"result","cache_control":{"type":"ephemeral"}}]}]"#
        )
    );
}

#[tokio::test]
async fn a_non_string_tool_call_id_reads_as_empty_and_a_null_cache_control_is_dropped() {
    let split = split_turns(
        r#"[{"role":"tool","tool_call_id":5,"content":"result","cache_control":null}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"","content":"result"}]}]"#
        )
    );
    let missing = split_turns(r#"[{"role":"tool","content":"result"}]"#).await;
    assert_eq!(
        conversation(&missing),
        parsed(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"","content":"result"}]}]"#
        )
    );
}

#[tokio::test]
async fn a_tool_content_list_goes_through_the_shared_converter() {
    let split = split_turns(
        r#"[{"role":"tool","tool_call_id":"call_1","content":[{"type":"text","text":"found","cache_control":{"type":"ephemeral"}},{"type":"image_url","image_url":"data:image/png;base64,AAA"},{"type":"image_url","image_url":"data:application/pdf;base64,PDF"},{"type":"tool_reference","tool_name":"f"}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":[{"type":"text","text":"found","cache_control":{"type":"ephemeral"}},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAA"}},{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"PDF"}},{"type":"tool_reference","tool_name":"f"}]}]}]"#
        )
    );
}

const REMOTE_FILE_TOOL_RESULT: &str = r#"[{"role":"tool","tool_call_id":"call_1","content":[{"type":"file","file":{"file_data":"https://example.test/report.pdf"}}]}]"#;

#[tokio::test]
async fn a_remote_file_part_in_a_tool_result_is_fetched_even_when_the_fetch_is_slow() {
    let source = MediaSource::slow(&[(
        "https://example.test/report.pdf",
        "data:application/pdf;base64,PDF",
    )]);
    let split = extract_system_and_messages(&messages(REMOTE_FILE_TOOL_RESULT), &source)
        .await
        .unwrap();
    assert_eq!(source.converted(), ["https://example.test/report.pdf"]);
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":[{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"PDF"}}]}]}]"#
        )
    );
}

#[tokio::test]
async fn a_local_file_part_in_a_tool_result_is_not_fetched() {
    let source = MediaSource::default();
    let split = extract_system_and_messages(
        &messages(
            r#"[{"role":"tool","tool_call_id":"call_1","content":[{"type":"file","file":{"file_data":"data:application/pdf;base64,PDF"}},{"type":"file","file":{"file_id":"file_1"}}]}]"#,
        ),
        &source,
    )
    .await
    .unwrap();
    assert!(source.converted().is_empty());
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_1","content":[{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"PDF"}},{"type":"container_upload","file_id":"file_1"}]}]}]"#
        )
    );
}

#[tokio::test]
async fn a_failed_file_fetch_in_a_tool_result_is_an_error() {
    let outcome =
        extract_system_and_messages(&messages(REMOTE_FILE_TOOL_RESULT), &MediaSource::default())
            .await;
    assert!(matches!(
        outcome,
        Err(MessageError::FileBlock(FileBlockError::Fetch(_)))
    ));
}

#[tokio::test]
async fn a_file_that_fetches_to_another_remote_url_is_not_ready_for_conversion() {
    let source = MediaSource::slow(&[(
        "https://example.test/report.pdf",
        "https://example.test/other.pdf",
    )]);
    let outcome = extract_system_and_messages(&messages(REMOTE_FILE_TOOL_RESULT), &source).await;
    assert!(matches!(
        outcome,
        Err(MessageError::ToolResult(ToolResultError::File(
            FileBlockError::Pending
        )))
    ));
}

#[tokio::test]
async fn consecutive_tool_results_join_one_user_message() {
    let split = split_turns(
        r#"[{"role":"tool","tool_call_id":"a","content":"one"},{"role":"tool","tool_call_id":"b","content":"two"}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"one"},{"type":"tool_result","tool_use_id":"b","content":"two"}]}]"#
        )
    );
}

#[tokio::test]
async fn a_user_turn_between_tool_results_starts_a_new_user_message() {
    let split = split_turns(
        r#"[{"role":"tool","tool_call_id":"a","content":"one"},{"role":"user","content":"hi"},{"role":"tool","tool_call_id":"b","content":"two"}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"tool_result","tool_use_id":"a","content":"one"}]},{"role":"user","content":"hi"},{"role":"user","content":[{"type":"tool_result","tool_use_id":"b","content":"two"}]}]"#
        )
    );
}

#[tokio::test]
async fn an_image_url_string_becomes_a_base64_image_block() {
    let split = split_turns(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"data:image/jpeg;base64,BBB"}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"BBB"}}]}]"#
        )
    );
}

#[tokio::test]
async fn an_image_url_object_uses_its_format_and_keeps_its_cache_control() {
    let split = split_turns(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"data:image/png;base64,CCC","format":"image/webp"},"cache_control":{"type":"ephemeral"}}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/webp","data":"CCC"},"cache_control":{"type":"ephemeral"}}]}]"#
        )
    );
}

#[tokio::test]
async fn a_remote_image_url_is_fetched_before_conversion() {
    let source =
        MediaSource::with(&[("https://example.test/cat.png", "data:image/png;base64,AAA")]);
    let split = extract_system_and_messages(
        &messages(
            r#"[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/cat.png"}}]}]"#,
        ),
        &source,
    )
    .await
    .unwrap();
    assert_eq!(source.converted(), ["https://example.test/cat.png"]);
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAA"}}]}]"#
        )
    );
}

#[tokio::test]
async fn a_user_image_url_with_a_document_data_uri_becomes_a_document_block() {
    let split = split_turns(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"data:application/pdf;base64,PDF","cache_control":{"type":"ephemeral"}},{"type":"image_url","image_url":{"url":"data:text/plain;base64,TXT"}},{"type":"image_url","image_url":"data:image/png;base64,AAA"}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"PDF"},"cache_control":{"type":"ephemeral"}},{"type":"document","source":{"type":"base64","media_type":"text/plain","data":"TXT"}},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAA"}}]}]"#
        )
    );
}

#[tokio::test]
async fn an_image_url_without_a_url_stays_as_it_is() {
    for part in [
        r#"{"type":"image_url","image_url":{}}"#,
        r#"{"type":"image_url","image_url":{"url":""}}"#,
        r#"{"type":"image_url","image_url":5}"#,
    ] {
        let source = format!(r#"[{{"role":"user","content":[{part}]}}]"#);
        let expected = format!(r#"[{{"role":"user","content":[{part}]}}]"#);
        let split = split_turns(&source).await;
        assert_eq!(conversation(&split), parsed(&expected));
    }
}

#[tokio::test]
async fn a_non_image_part_in_a_user_list_passes_through() {
    let split = split_turns(
        r#"[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"input_audio","input_audio":{"data":"x"}}]}]"#,
    )
    .await;
    assert_eq!(
        conversation(&split),
        parsed(
            r#"[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"input_audio","input_audio":{"data":"x"}}]}]"#
        )
    );
}

#[test]
fn a_function_tool_becomes_name_description_and_cleaned_schema() {
    assert_eq!(
        Json::List(
            transform_tools(&tool_list(
                r#"[{"type":"function","function":{"name":"f","description":"d","parameters":{"$schema":"s","type":"object","properties":{}}}}]"#
            ))
            .unwrap()
        ),
        parsed(r#"[{"name":"f","description":"d","input_schema":{"type":"object","properties":{}}}]"#)
    );
}

#[test]
fn a_function_tool_defaults_its_name_and_schema() {
    assert_eq!(
        Json::List(transform_tools(&tool_list(r#"[{"type":"function","function":{}}]"#)).unwrap()),
        parsed(r#"[{"name":"","input_schema":{"type":"object","properties":{}}}]"#)
    );
}

#[test]
fn a_non_function_tool_cleans_a_present_schema_and_keeps_the_rest() {
    assert_eq!(
        Json::List(
            transform_tools(&tool_list(
                r#"[{"type":"custom","input_schema":{"$schema":"s","type":"object"}},{"type":"other","name":"n"}]"#
            ))
            .unwrap()
        ),
        parsed(r#"[{"type":"custom","input_schema":{"type":"object"}},{"type":"other","name":"n"}]"#)
    );
}

#[test]
fn a_tool_that_is_not_an_object_is_an_error() {
    let error = transform_tools(&tool_list("[5]")).unwrap_err();
    assert_eq!(error.to_string(), "tools must be a list of objects");
}

#[test]
fn tool_choice_strings_map_to_anthropic_types() {
    for (source, expected) in [
        (r#""auto""#, r#"{"type":"auto"}"#),
        (r#""required""#, r#"{"type":"any"}"#),
        (r#""none""#, r#"{"type":"none"}"#),
        (r#""unknown""#, r#"{"type":"auto"}"#),
        ("5", r#"{"type":"auto"}"#),
    ] {
        assert_eq!(
            transform_tool_choice(&parsed(source)).unwrap(),
            parsed(expected)
        );
    }
}

#[test]
fn a_function_tool_choice_becomes_a_tool_and_other_dicts_pass_through() {
    assert_eq!(
        transform_tool_choice(&parsed(r#"{"type":"function","function":{"name":"f"}}"#)).unwrap(),
        parsed(r#"{"type":"tool","name":"f"}"#)
    );
    assert_eq!(
        transform_tool_choice(&parsed(r#"{"type":"auto"}"#)).unwrap(),
        parsed(r#"{"type":"auto"}"#)
    );
    let error = transform_tool_choice(&parsed(r#"{"type":"function","function":5}"#)).unwrap_err();
    assert!(matches!(error, ToolError::ChoiceFunctionNotObject));
}

#[tokio::test]
async fn a_message_with_a_non_object_function_is_an_error() {
    let outcome = extract_system_and_messages(
        &messages(r#"[{"role":"assistant","content":"x","tool_calls":[{"id":"a","function":5}]}]"#),
        &MediaSource::default(),
    )
    .await;
    assert!(matches!(
        outcome,
        Err(MessageError::Field {
            field: "function",
            expected: "an object"
        })
    ));
}
