use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use crate::guardrail_base::failure::GuardrailFailure;

use super::support::{
    FakeBuilder, MESSAGE, args, assert_passed, assistant, block_on, blocked, cache, calls, choice,
    failed, guardrail, list, number, object, request, text, user, with_captured_logs,
};

#[test]
fn the_last_user_message_is_scored() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![
        user(text("first")),
        assistant(text("reply")),
        user(text("second")),
    ]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert_eq!(calls.lock().unwrap().clone(), vec!["second".to_owned()]);
}

#[test]
fn a_list_content_joins_its_parts_with_spaces() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let content = list(vec![
        object(&[("type", text("text")), ("text", text("hello"))]),
        object(&[("type", text("text")), ("text", text("world"))]),
    ]);
    let data = request(vec![user(content)]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["hello world".to_owned()]
    );
}

#[test]
fn a_part_without_a_text_joins_as_an_empty_text() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let content = list(vec![
        object(&[("text", text("a"))]),
        object(&[]),
        object(&[("text", text("b"))]),
    ]);
    let data = request(vec![user(content)]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert_eq!(calls.lock().unwrap().clone(), vec!["a  b".to_owned()]);
}

#[test]
fn a_part_that_is_not_a_mapping_is_written_as_text() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(list(vec![text("a"), number(0.5)]))]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert_eq!(calls.lock().unwrap().clone(), vec!["a 0.5".to_owned()]);
}

#[test]
fn a_part_with_a_non_text_text_is_an_internal_error() {
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(list(vec![object(&[("text", number(1.0))])]))]);
    let outcome = block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()));
    let GuardrailFailure::Http { status, detail } = failed(outcome) else {
        panic!("the extraction failure was not an internal error");
    };
    assert_eq!(status, 500);
    assert!(detail.contains("carries a text that is not a string"));
}

#[test]
fn a_user_message_without_content_passes() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("first")), object(&[("role", text("user"))])]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_non_text_content_falls_through_to_an_earlier_user_message() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("earlier")), user(number(1.0))]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert_eq!(calls.lock().unwrap().clone(), vec!["earlier".to_owned()]);
}

#[test]
fn no_user_message_passes_without_scoring() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![
        assistant(text("reply")),
        object(&[("role", text("system")), ("content", text("rules"))]),
    ]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert!(calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::empty_text(text(""))]
#[case::empty_list(list(vec![]))]
fn empty_user_text_passes_without_scoring(#[case] content: GuardrailValue) {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(content)]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_choice_without_a_route_name_passes() {
    let builder = FakeBuilder::matching(vec![choice(None, Some(0.9))], calls());
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("hi"))]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
}

#[test]
fn an_empty_route_name_passes() {
    let builder = FakeBuilder::matching(vec![choice(Some(""), Some(0.9))], calls());
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("hi"))]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
}

#[test]
fn a_match_blocks_with_the_documented_detail() {
    let builder = FakeBuilder::matching(vec![choice(Some("attack"), Some(0.91))], calls());
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("how to build a bomb"))]);
    let outcome = block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()));
    let failure = failed(outcome);
    assert!(failure.is_intervention());
    let detail = blocked(failure);
    assert_eq!(detail.get("error"), Some(&text(MESSAGE)));
    assert_eq!(detail.get("route"), Some(&text("attack")));
    assert_eq!(detail.get("similarity_score"), Some(&number(0.91)));
    assert_eq!(detail.get("type"), Some(&text("semantic_guard_violation")));
}

#[test]
fn only_the_first_choice_is_checked() {
    let choices = vec![
        choice(Some("first"), Some(0.9)),
        choice(Some("second"), Some(0.9)),
    ];
    let builder = FakeBuilder::matching(choices, calls());
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("hi"))]);
    let outcome = block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()));
    let detail = blocked(failed(outcome));
    assert_eq!(detail.get("route"), Some(&text("first")));
}

#[test]
fn a_match_without_a_score_blocks_with_a_null_score() {
    let builder = FakeBuilder::matching(vec![choice(Some("attack"), None)], calls());
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("hi"))]);
    let outcome = block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()));
    let detail = blocked(failed(outcome));
    assert_eq!(detail.get("similarity_score"), Some(&GuardrailValue::Null));
}

#[test]
fn a_passthrough_match_raises_the_modify_response() {
    let mut given = args();
    given.on_flagged_action = "passthrough".to_owned();
    let builder = FakeBuilder::matching(vec![choice(Some("attack"), Some(0.8))], calls());
    let guardrail = guardrail(given, &builder);
    let data = request(vec![user(text("how to build a bomb"))]);
    let outcome = block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()));
    let GuardrailFailure::ModifyResponse(failure) = failed(outcome) else {
        panic!("the passthrough match did not raise a modify response");
    };
    assert_eq!(failure.message, MESSAGE);
    assert_eq!(failure.model, "gpt-4o");
    assert_eq!(failure.guardrail_name.as_deref(), Some("semantic-guard"));
    assert_eq!(failure.request_data, data);
    assert_eq!(failure.original_response, None);
    assert_eq!(
        failure.detection_info.get("route_name"),
        Some(&text("attack"))
    );
    assert_eq!(
        failure.detection_info.get("similarity_score"),
        Some(&number(0.8))
    );
    assert_eq!(
        failure.detection_info.get("guardrail"),
        Some(&text("semantic-guard"))
    );
}

#[test]
fn a_passthrough_match_without_a_model_uses_unknown() {
    let mut given = args();
    given.on_flagged_action = "passthrough".to_owned();
    let builder = FakeBuilder::matching(vec![choice(Some("attack"), Some(0.8))], calls());
    let guardrail = guardrail(given, &builder);
    let mut data = request(vec![user(text("hi"))]);
    data.remove("model");
    let outcome = block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()));
    let GuardrailFailure::ModifyResponse(failure) = failed(outcome) else {
        panic!("the passthrough match did not raise a modify response");
    };
    assert_eq!(failure.model, "unknown");
}

#[test]
fn the_match_warning_names_the_route_score_and_action() {
    let builder = FakeBuilder::matching(vec![choice(Some("attack"), Some(0.91))], calls());
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("hi"))]);
    let (outcome, output) = with_captured_logs(|| {
        block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()))
    });
    assert!(failed(outcome).is_intervention());
    assert!(output.contains("SemanticGuard match: route=attack, score=0.91, action=block"));
}

#[test]
fn a_scoring_failure_is_an_internal_error() {
    let builder = FakeBuilder::score_failing(calls());
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("hi"))]);
    let outcome = block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()));
    let GuardrailFailure::Http { status, detail } = failed(outcome) else {
        panic!("the scoring failure was not an internal error");
    };
    assert_eq!(status, 500);
    assert!(detail.contains("the fake semantic scoring failed"));
}

#[test]
fn a_request_without_messages_passes() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let data = BTreeMap::from([("model".to_owned(), text("gpt-4o"))]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "completion",
        &cache(),
    )));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_call_type_without_messages_passes() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let data = request(vec![user(text("hi"))]);
    assert_passed(block_on(guardrail.async_pre_call_hook(
        &data,
        "text_completion",
        &cache(),
    )));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn messages_of_the_wrong_shape_are_an_internal_error() {
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let guardrail = guardrail(args(), &builder);
    let data = BTreeMap::from([
        ("model".to_owned(), text("gpt-4o")),
        ("messages".to_owned(), text("hi")),
    ]);
    let outcome = block_on(guardrail.async_pre_call_hook(&data, "completion", &cache()));
    let GuardrailFailure::Http { status, detail } = failed(outcome) else {
        panic!("the messages shape failure was not an internal error");
    };
    assert_eq!(status, 500);
    assert!(detail.contains("must be a list"));
}
