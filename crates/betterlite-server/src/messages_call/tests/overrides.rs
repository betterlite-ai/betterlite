#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::{
    FakeDispatch, Flag, MessagesRoute, call_steps, empty_routing, input, object,
    plan_messages_route, router_plan, text,
};

#[test]
fn the_team_map_wins_over_the_model_lists() {
    let mut routing = empty_routing();
    routing.team_model = Some("internal".to_owned());
    let mut input = input(Some("public"), true);
    input.team_id = Some("team".to_owned());
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        router_plan(false)
    );
    let team_queries = routing.team_queries.lock().unwrap();
    let expected = [(Some("public".to_owned()), "team".to_owned())];
    assert_eq!(team_queries.as_slice(), expected.as_slice());
}

#[test]
fn the_blocked_and_missing_model_checks_run_before_the_team_map() {
    let mut routing = empty_routing();
    routing.team_model = Some("internal".to_owned());
    routing.blocked = Flag(true);
    let mut blocked = input(Some("public"), true);
    blocked.team_id = Some("team".to_owned());
    assert_eq!(
        plan_messages_route(&blocked, Some(&routing)),
        MessagesRoute::Blocked
    );
    assert!(routing.team_queries.lock().unwrap().is_empty());
    routing.blocked = Flag(false);
    let mut missing = input(None, false);
    missing.team_id = Some("team".to_owned());
    assert_eq!(
        plan_messages_route(&missing, Some(&routing)),
        MessagesRoute::MissingModel
    );
    assert!(routing.team_queries.lock().unwrap().is_empty());
}

fn values(prefix: &str, retries: i64, timeout: i64, retry_policy: i64, tag: bool) -> JsonMap {
    object(&[
        ("model", text("m")),
        (
            "fallbacks",
            JsonValue::Array(vec![text(&format!("{prefix}-fallback"))]),
        ),
        (
            "context_window_fallbacks",
            JsonValue::Array(vec![text(&format!("{prefix}-context"))]),
        ),
        (
            "content_policy_fallbacks",
            JsonValue::Array(vec![text(&format!("{prefix}-policy"))]),
        ),
        ("num_retries", JsonValue::from(retries)),
        ("timeout", JsonValue::from(timeout)),
        (
            "model_group_retry_policy",
            JsonValue::Object(object(&[("retries", JsonValue::from(retry_policy))])),
        ),
        ("routing_strategy", text(&format!("{prefix}-strategy"))),
        ("enable_tag_filtering", JsonValue::Bool(tag)),
    ])
}

fn assert_sent(sent: &JsonMap, expected: &JsonMap) {
    assert!(!sent.contains_key("router_settings_override"));
    for (key, value) in expected {
        assert_eq!(sent.get(key), Some(value), "{key}");
    }
}

#[tokio::test]
async fn the_override_copies_only_the_keys_the_body_lacks() {
    let routing = Arc::new(empty_routing());
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&routing)),
        None,
        false,
        None,
    );
    let expected = values("override", 2, 5, 1, true);
    let data = object(&[
        ("model", text("m")),
        (
            "router_settings_override",
            JsonValue::Object(expected.clone()),
        ),
    ]);
    steps.call(data, false).await.unwrap();
    let sent = routing.router_calls.lock().unwrap()[0].0.clone();
    assert_sent(&sent, &expected);
}

#[tokio::test]
async fn the_body_values_keep_the_override_from_writing_them() {
    let routing = Arc::new(empty_routing());
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&routing)),
        None,
        false,
        None,
    );
    let mut body = values("body", 4, 9, 3, false);
    body.insert(
        "router_settings_override".to_owned(),
        JsonValue::Object(values("override", 2, 5, 1, true)),
    );
    steps.call(body, false).await.unwrap();
    let sent = routing.router_calls.lock().unwrap()[0].0.clone();
    // The shared step pops the body's flag before the override copy, so the override's value wins
    assert_sent(&sent, &values("body", 4, 9, 3, true));
}

fn team_metadata() -> JsonValue {
    JsonValue::Object(object(&[("user_api_key_team_id", text("team"))]))
}

#[tokio::test]
async fn the_team_map_is_written_into_the_router_call_only_from_the_router_branch() {
    let mut fake = empty_routing();
    fake.team_model = Some("internal".to_owned());
    let routing = Arc::new(fake);
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&routing)),
        None,
        false,
        None,
    );
    let data = object(&[("model", text("public")), ("metadata", team_metadata())]);
    steps.call(data, false).await.unwrap();
    let sent = routing.router_calls.lock().unwrap()[0].0.clone();
    assert_eq!(sent.get("model"), Some(&text("internal")));
    let keyed = object(&[
        ("model", text("public")),
        ("api_key", text("k")),
        ("metadata", team_metadata()),
    ]);
    steps.call(keyed, false).await.unwrap();
    let keyed_sent = routing.router_calls.lock().unwrap()[1].0.clone();
    assert_eq!(keyed_sent.get("model"), Some(&text("public")));
    assert_eq!(routing.team_queries.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn the_override_branch_does_not_ask_the_team_map() {
    let mut fake = empty_routing();
    fake.team_model = Some("internal".to_owned());
    let routing = Arc::new(fake);
    let steps = call_steps(
        Arc::new(FakeDispatch::default()),
        Some(Arc::clone(&routing)),
        None,
        false,
        None,
    );
    let overridden = object(&[
        ("model", text("public")),
        (
            "router_settings_override",
            JsonValue::Object(JsonMap::new()),
        ),
        ("metadata", team_metadata()),
    ]);
    steps.call(overridden, false).await.unwrap();
    let sent = routing.router_calls.lock().unwrap()[0].0.clone();
    assert_eq!(sent.get("model"), Some(&text("public")));
    assert!(routing.team_queries.lock().unwrap().is_empty());
}
