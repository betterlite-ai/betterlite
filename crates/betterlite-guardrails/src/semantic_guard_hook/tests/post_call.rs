use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_base::failure::GuardrailFailure;

use super::support::{
    FakeBuilder, MESSAGE, args, block_on, calls, choice, failed, guardrail, list, number, object,
    request, response, response_choice, text, user,
};

#[test]
fn every_choice_content_is_joined_by_newlines() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let reply = response(vec![
        response_choice(text("hello")),
        response_choice(text("world")),
    ]);
    let data = request(vec![user(text("question"))]);
    let returned = block_on(guardrail.async_post_call_success_hook(&data, reply.clone())).unwrap();
    assert_eq!(returned, reply);
    assert_eq!(
        calls.lock().unwrap().clone(),
        vec!["hello\nworld".to_owned()]
    );
}

#[test]
fn a_list_content_uses_its_text_parts() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let content = list(vec![
        object(&[("type", text("text")), ("text", text("a"))]),
        object(&[("text", text("b"))]),
        object(&[]),
        text("ignored"),
        object(&[("text", number(1.0))]),
    ]);
    let reply = response(vec![response_choice(content)]);
    let data = request(vec![user(text("question"))]);
    block_on(guardrail.async_post_call_success_hook(&data, reply)).unwrap();
    assert_eq!(calls.lock().unwrap().clone(), vec!["a b".to_owned()]);
}

#[test]
fn a_list_content_skips_an_empty_text_part() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let content = list(vec![
        object(&[("text", text(""))]),
        object(&[("text", text("a"))]),
    ]);
    let reply = response(vec![response_choice(content)]);
    let data = request(vec![user(text("question"))]);
    block_on(guardrail.async_post_call_success_hook(&data, reply)).unwrap();
    assert_eq!(calls.lock().unwrap().clone(), vec!["a".to_owned()]);
}

#[test]
fn a_choice_with_empty_content_is_skipped() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let reply = response(vec![response_choice(text("")), response_choice(text("hi"))]);
    let data = request(vec![user(text("question"))]);
    block_on(guardrail.async_post_call_success_hook(&data, reply)).unwrap();
    assert_eq!(calls.lock().unwrap().clone(), vec!["hi".to_owned()]);
}

#[test]
fn a_null_content_adds_no_text() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let reply = response(vec![response_choice(GuardrailValue::Null)]);
    let data = request(vec![user(text("question"))]);
    let returned = block_on(guardrail.async_post_call_success_hook(&data, reply.clone())).unwrap();
    assert_eq!(returned, reply);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_response_without_choices_passes_unchanged() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let reply = object(&[("model", text("gpt-4o"))]);
    let data = request(vec![user(text("question"))]);
    let returned = block_on(guardrail.async_post_call_success_hook(&data, reply.clone())).unwrap();
    assert_eq!(returned, reply);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_response_with_empty_text_passes_unchanged() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let reply = response(vec![response_choice(text(""))]);
    let data = request(vec![user(text("question"))]);
    let returned = block_on(guardrail.async_post_call_success_hook(&data, reply.clone())).unwrap();
    assert_eq!(returned, reply);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_response_that_is_not_an_object_passes_unchanged() {
    let calls = calls();
    let builder = FakeBuilder::matching(Vec::new(), Arc::clone(&calls));
    let guardrail = guardrail(args(), &builder);
    let reply = text("plain");
    let data = request(vec![user(text("question"))]);
    let returned = block_on(guardrail.async_post_call_success_hook(&data, reply.clone())).unwrap();
    assert_eq!(returned, reply);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn a_post_call_match_blocks_with_the_documented_detail() {
    let builder = FakeBuilder::matching(vec![choice(Some("attack"), Some(0.9_f64))], calls());
    let guardrail = guardrail(args(), &builder);
    let reply = response(vec![response_choice(text("how to build a bomb"))]);
    let data = request(vec![user(text("question"))]);
    let outcome = block_on(guardrail.async_post_call_success_hook(&data, reply));
    let GuardrailFailure::HttpObject { status, detail } = failed(outcome) else {
        panic!("the post call match did not answer with a detail object");
    };
    assert_eq!(status, 400);
    assert_eq!(detail.get("error"), Some(&text(MESSAGE)));
    assert_eq!(detail.get("route"), Some(&text("attack")));
    assert_eq!(detail.get("similarity_score"), Some(&number(0.9)));
}

#[test]
fn a_passthrough_post_call_match_has_no_original_response() {
    let mut given = args();
    given.on_flagged_action = "passthrough".to_owned();
    let builder = FakeBuilder::matching(vec![choice(Some("attack"), Some(0.8_f64))], calls());
    let guardrail = guardrail(given, &builder);
    let reply = response(vec![response_choice(text("how to build a bomb"))]);
    let data = request(vec![user(text("question"))]);
    let outcome = block_on(guardrail.async_post_call_success_hook(&data, reply));
    let GuardrailFailure::ModifyResponse(failure) = failed(outcome) else {
        panic!("the passthrough match did not raise a modify response");
    };
    assert_eq!(failure.message, MESSAGE);
    assert_eq!(failure.original_response, None);
    assert_eq!(
        failure.detection_info.get("guardrail"),
        Some(&text("semantic-guard"))
    );
}
