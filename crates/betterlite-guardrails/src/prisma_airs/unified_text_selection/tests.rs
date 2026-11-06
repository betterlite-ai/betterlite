#![cfg(test)]

use std::collections::BTreeSet;

use rstest::rstest;

use super::{
    is_anthropic_request, latest_user_text_indices, reasoning_item_text_indices,
    role_filtered_text_indices, text_source_message_indices, use_latest_user_only,
};
use crate::panw_prisma_airs::audit::Direction;
use crate::prisma_airs::request_data::{ProxyServerRequest, ResponsesInput};
use crate::prisma_airs::support::{
    Run, build_guardrail, execute, inputs, logging, messages, prompt_json, request_data,
    response_json, responses_input, settings, texts,
};

const ALLOW: &str = r#"{"action":"allow","category":"benign","scan_id":"scan-1"}"#;

#[test]
fn every_text_maps_to_the_message_it_came_from() {
    let given = texts(&["sys", "u1", "a1"]);
    let given_messages = messages(
        r#"[{"role":"system","content":"sys"},{"role":"user","content":"u1"},{"role":"assistant","content":"a1"}]"#,
    );
    assert_eq!(
        text_source_message_indices(&given, &given_messages),
        Some(vec![0, 1, 2])
    );
}

#[test]
fn messages_without_counterpart_texts_are_skipped() {
    let given = texts(&["hello", "bye"]);
    let given_messages = messages(
        r#"[{"role":"user","content":"hello"},{"role":"tool","content":null},{"role":"user","content":"bye"}]"#,
    );
    assert_eq!(
        text_source_message_indices(&given, &given_messages),
        Some(vec![0, 2])
    );
}

#[test]
fn a_content_list_flattens_to_one_text_per_text_part() {
    let given = texts(&["a", "b"]);
    let given_messages = messages(
        r#"[{"role":"user","content":[{"type":"text","text":"a"},{"type":"image_url","image_url":{"url":"x"}},{"type":"text","text":"b"}]}]"#,
    );
    assert_eq!(
        text_source_message_indices(&given, &given_messages),
        Some(vec![0, 0])
    );
}

#[test]
fn disagreeing_walks_fall_back_to_scanning_everything() {
    let given = texts(&["x", "y"]);
    let given_messages = messages(
        r#"[{"role":"user","content":[{"type":"text","text":"x"},{"type":"text","text":"y"}]},{"role":"user","content":[{"type":"text","text":"y"}]}]"#,
    );
    assert_eq!(text_source_message_indices(&given, &given_messages), None);
}

#[test]
fn the_role_filter_keeps_user_system_and_developer_texts() {
    let given = texts(&["s", "u", "a", "d", "t"]);
    let given_messages = messages(
        r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"},{"role":"developer","content":"d"},{"role":"tool","content":"t"}]"#,
    );
    assert_eq!(
        role_filtered_text_indices(&given, &given_messages),
        Some(BTreeSet::from([0, 1, 3]))
    );
}

#[test]
fn a_reasoning_item_after_the_turn_is_not_the_latest_human_message() {
    let mut data = request_data();
    data.input = Some(responses_input(
        r#"[{"type":"message","role":"user","content":"old"},{"type":"reasoning","content":[{"type":"text","text":"think"}]}]"#,
    ));
    let given = texts(&["old", "think"]);
    let given_messages =
        messages(r#"[{"role":"user","content":"old"},{"role":"user","content":"think"}]"#);
    assert_eq!(
        latest_user_text_indices(&given, &given_messages, &data),
        Some(BTreeSet::from([0]))
    );
}

#[test]
fn a_latest_message_without_text_leaves_nothing_to_scan() {
    let given = texts(&["old"]);
    let given_messages =
        messages(r#"[{"role":"user","content":"old"},{"role":"user","content":[]}]"#);
    assert_eq!(
        latest_user_text_indices(&given, &given_messages, &request_data()),
        Some(BTreeSet::new())
    );
}

#[test]
fn a_latest_text_that_never_reached_the_texts_falls_back_to_the_role_filter() {
    let given = texts(&["old"]);
    let given_messages =
        messages(r#"[{"role":"user","content":"old"},{"role":"user","content":"new"}]"#);
    assert_eq!(
        latest_user_text_indices(&given, &given_messages, &request_data()),
        None
    );
}

#[test]
fn reasoning_indices_include_the_instructions_offset() {
    let mut data = request_data();
    data.instructions = Some("sys".to_owned());
    data.input = Some(responses_input(
        r#"[{"type":"message","role":"user","content":"hi"},{"type":"reasoning","content":[{"type":"text","text":"think"}]}]"#,
    ));
    assert_eq!(
        reasoning_item_text_indices(&texts(&["sys", "hi", "think"]), &data),
        Some(BTreeSet::from([2]))
    );
    assert_eq!(
        reasoning_item_text_indices(&texts(&["sys", "hi"]), &data),
        None
    );
}

#[rstest]
#[case::no_input(None, Some(BTreeSet::new()))]
#[case::string_input(Some(ResponsesInput::Text("hi".to_owned())), Some(BTreeSet::new()))]
fn an_input_without_items_has_no_reasoning_indices(
    #[case] input: Option<ResponsesInput>,
    #[case] expected: Option<BTreeSet<usize>>,
) {
    let mut data = request_data();
    data.input = input;
    assert_eq!(
        reasoning_item_text_indices(&texts(&["hi"]), &data),
        expected
    );
}

#[test]
fn anthropic_detection_uses_the_logging_call_type() {
    let mut logging_obj = logging();
    logging_obj.call_type = Some("anthropic_messages".to_owned());
    assert!(is_anthropic_request(&request_data(), Some(&logging_obj)));
    logging_obj.call_type = Some("completion".to_owned());
    assert!(!is_anthropic_request(&request_data(), Some(&logging_obj)));
}

#[rstest]
#[case::exact("https://proxy.example/v1/messages", true)]
#[case::trailing_slash("https://proxy.example/v1/messages/", true)]
#[case::query("https://proxy.example/v1/messages?beta=1", true)]
#[case::nested_path("https://proxy.example/anthropic/v1/messages", true)]
#[case::batch_suffix("https://proxy.example/v1/messages_batch", false)]
#[case::other_path("https://proxy.example/other", false)]
#[case::empty_url("", false)]
fn anthropic_detection_uses_the_request_url(#[case] url: &str, #[case] expected: bool) {
    let mut data = request_data();
    data.proxy_server_request = Some(ProxyServerRequest {
        url: Some(url.to_owned()),
    });
    assert_eq!(is_anthropic_request(&data, None), expected);
}

#[test]
fn the_latest_only_flag_wins_over_the_anthropic_default() {
    let mut data = request_data();
    data.proxy_server_request = Some(ProxyServerRequest {
        url: Some("https://proxy.example/v1/messages".to_owned()),
    });
    let mut off = settings("http://localhost");
    off.latest_role_message_only = Some(false);
    let off_guardrail = build_guardrail(off);
    assert!(!use_latest_user_only(&off_guardrail, &data, None));
    let mut on = settings("http://localhost");
    on.latest_role_message_only = Some(true);
    let on_guardrail = build_guardrail(on);
    assert!(use_latest_user_only(&on_guardrail, &request_data(), None));
}

#[test]
fn an_anthropic_url_selects_latest_only_by_default() {
    let mut data = request_data();
    data.proxy_server_request = Some(ProxyServerRequest {
        url: Some("https://proxy.example/v1/messages".to_owned()),
    });
    let guardrail = build_guardrail(settings("http://localhost"));
    assert!(use_latest_user_only(&guardrail, &data, None));
    assert!(!use_latest_user_only(&guardrail, &request_data(), None));
}

#[tokio::test]
async fn a_request_scan_skips_assistant_and_tool_history() {
    let mut given = inputs(&["sys", "user", "assistant", "tool"]);
    given.structured_messages = messages(
        r#"[{"role":"system","content":"sys"},{"role":"user","content":"user"},{"role":"assistant","content":"assistant"},{"role":"tool","content":"tool"}]"#,
    );
    let outcome = execute(Run::new(ALLOW), given, request_data(), None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.bodies.len(), 2);
    assert!(outcome.bodies[0].contains(&prompt_json("sys")));
    assert!(outcome.bodies[1].contains(&prompt_json("user")));
}

#[tokio::test]
async fn an_anthropic_request_scans_only_the_latest_user_message() {
    let mut given = inputs(&["old", "new"]);
    given.structured_messages =
        messages(r#"[{"role":"user","content":"old"},{"role":"user","content":"new"}]"#);
    let mut data = request_data();
    data.proxy_server_request = Some(ProxyServerRequest {
        url: Some("https://proxy.example/v1/messages".to_owned()),
    });
    let outcome = execute(Run::new(ALLOW), given, data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.bodies.len(), 1);
    assert!(outcome.bodies[0].contains(&prompt_json("new")));
}

#[tokio::test]
async fn a_responses_reasoning_item_is_never_the_latest_turn() {
    let mut given = inputs(&["old", "think"]);
    given.structured_messages =
        messages(r#"[{"role":"user","content":"old"},{"role":"user","content":"think"}]"#);
    let mut data = request_data();
    data.input = Some(responses_input(
        r#"[{"type":"message","role":"user","content":"old"},{"type":"reasoning","content":[{"type":"text","text":"think"}]}]"#,
    ));
    let run = Run {
        latest_only: Some(true),
        ..Run::new(ALLOW)
    };
    let outcome = execute(run, given, data, None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.bodies.len(), 1);
    assert!(outcome.bodies[0].contains(&prompt_json("old")));
}

#[tokio::test]
async fn a_latest_turn_without_text_leaves_nothing_to_scan() {
    let mut given = inputs(&["old"]);
    given.structured_messages =
        messages(r#"[{"role":"user","content":"old"},{"role":"user","content":[]}]"#);
    let run = Run {
        latest_only: Some(true),
        ..Run::new(ALLOW)
    };
    let outcome = execute(run, given, request_data(), None).await;
    outcome.result.as_ref().expect("allowed");
    assert!(outcome.bodies.is_empty());
    assert_eq!(outcome.inputs.texts, ["old"]);
}

#[tokio::test]
async fn a_misaligned_latest_turn_falls_back_to_the_scannable_roles() {
    let mut given = inputs(&["sys", "old", "answer"]);
    given.structured_messages = messages(
        r#"[{"role":"system","content":"sys"},{"role":"user","content":"old"},{"role":"assistant","content":"answer"},{"role":"user","content":"new"}]"#,
    );
    let run = Run {
        latest_only: Some(true),
        ..Run::new(ALLOW)
    };
    let outcome = execute(run, given, request_data(), None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.bodies.len(), 2);
    assert!(outcome.bodies[0].contains(&prompt_json("sys")));
    assert!(outcome.bodies[1].contains(&prompt_json("old")));
}

#[tokio::test]
async fn a_response_scan_ignores_the_text_selection() {
    let mut given = inputs(&["sys", "user", "assistant"]);
    given.structured_messages = messages(
        r#"[{"role":"system","content":"sys"},{"role":"user","content":"user"},{"role":"assistant","content":"assistant"}]"#,
    );
    let run = Run {
        direction: Direction::Response,
        ..Run::new(ALLOW)
    };
    let outcome = execute(run, given, request_data(), None).await;
    outcome.result.as_ref().expect("allowed");
    assert_eq!(outcome.bodies.len(), 3);
    assert!(outcome.bodies[2].contains(&response_json("assistant")));
}
