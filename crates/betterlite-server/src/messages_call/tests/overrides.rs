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
    let overrides = object(&[
        (
            "fallbacks",
            JsonValue::Array(vec![text("override-fallback")]),
        ),
        (
            "context_window_fallbacks",
            JsonValue::Array(vec![text("override-context")]),
        ),
        (
            "content_policy_fallbacks",
            JsonValue::Array(vec![text("override-policy")]),
        ),
        ("num_retries", JsonValue::from(2_i64)),
        ("timeout", JsonValue::from(5_i64)),
        (
            "model_group_retry_policy",
            JsonValue::Object(object(&[("retries", JsonValue::from(1_i64))])),
        ),
        ("routing_strategy", text("least-busy")),
        ("enable_tag_filtering", JsonValue::Bool(true)),
    ]);
    let data = object(&[
        ("model", text("m")),
        (
            "router_settings_override",
            JsonValue::Object(overrides.clone()),
        ),
    ]);
    steps.call(data, false).await.unwrap();
    {
        let calls = routing.router_calls.lock().unwrap();
        let sent = &calls[0].0;
        assert!(!sent.contains_key("router_settings_override"));
        assert_eq!(
            sent.get("fallbacks"),
            Some(&JsonValue::Array(vec![text("override-fallback")]))
        );
        assert_eq!(
            sent.get("context_window_fallbacks"),
            Some(&JsonValue::Array(vec![text("override-context")]))
        );
        assert_eq!(
            sent.get("content_policy_fallbacks"),
            Some(&JsonValue::Array(vec![text("override-policy")]))
        );
        assert_eq!(sent.get("num_retries"), Some(&JsonValue::from(2_i64)));
        assert_eq!(sent.get("timeout"), Some(&JsonValue::from(5_i64)));
        assert_eq!(
            sent.get("model_group_retry_policy"),
            Some(&JsonValue::Object(object(&[(
                "retries",
                JsonValue::from(1_i64)
            )])))
        );
        assert_eq!(sent.get("routing_strategy"), Some(&text("least-busy")));
        assert_eq!(
            sent.get("enable_tag_filtering"),
            Some(&JsonValue::Bool(true))
        );
    }
    let body = object(&[
        ("model", text("m")),
        ("fallbacks", JsonValue::Array(vec![text("body-fallback")])),
        (
            "context_window_fallbacks",
            JsonValue::Array(vec![text("body-context")]),
        ),
        (
            "content_policy_fallbacks",
            JsonValue::Array(vec![text("body-policy")]),
        ),
        ("num_retries", JsonValue::from(4_i64)),
        ("timeout", JsonValue::from(9_i64)),
        (
            "model_group_retry_policy",
            JsonValue::Object(object(&[("retries", JsonValue::from(3_i64))])),
        ),
        ("routing_strategy", text("body-strategy")),
        ("enable_tag_filtering", JsonValue::Bool(false)),
        ("router_settings_override", JsonValue::Object(overrides)),
    ]);
    steps.call(body, false).await.unwrap();
    let calls = routing.router_calls.lock().unwrap();
    let sent = &calls[1].0;
    assert_eq!(
        sent.get("fallbacks"),
        Some(&JsonValue::Array(vec![text("body-fallback")]))
    );
    assert_eq!(
        sent.get("context_window_fallbacks"),
        Some(&JsonValue::Array(vec![text("body-context")]))
    );
    assert_eq!(
        sent.get("content_policy_fallbacks"),
        Some(&JsonValue::Array(vec![text("body-policy")]))
    );
    assert_eq!(sent.get("num_retries"), Some(&JsonValue::from(4_i64)));
    assert_eq!(sent.get("timeout"), Some(&JsonValue::from(9_i64)));
    assert_eq!(
        sent.get("model_group_retry_policy"),
        Some(&JsonValue::Object(object(&[(
            "retries",
            JsonValue::from(3_i64)
        )])))
    );
    assert_eq!(sent.get("routing_strategy"), Some(&text("body-strategy")));
    // The shared step pops the body's flag before the override copy, so the override's value is the one kept
    assert_eq!(
        sent.get("enable_tag_filtering"),
        Some(&JsonValue::Bool(true))
    );
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
    let data = object(&[
        ("model", text("public")),
        (
            "metadata",
            JsonValue::Object(object(&[("user_api_key_team_id", text("team"))])),
        ),
    ]);
    steps.call(data, false).await.unwrap();
    {
        let calls = routing.router_calls.lock().unwrap();
        assert_eq!(calls[0].0.get("model"), Some(&text("internal")));
    }
    let keyed = object(&[
        ("model", text("public")),
        ("api_key", text("k")),
        (
            "metadata",
            JsonValue::Object(object(&[("user_api_key_team_id", text("team"))])),
        ),
    ]);
    steps.call(keyed, false).await.unwrap();
    {
        let calls = routing.router_calls.lock().unwrap();
        assert_eq!(calls[1].0.get("model"), Some(&text("public")));
    }
    assert_eq!(routing.team_queries.lock().unwrap().len(), 2);
    let overridden = object(&[
        ("model", text("public")),
        (
            "router_settings_override",
            JsonValue::Object(JsonMap::new()),
        ),
        (
            "metadata",
            JsonValue::Object(object(&[("user_api_key_team_id", text("team"))])),
        ),
    ]);
    steps.call(overridden, false).await.unwrap();
    {
        let calls = routing.router_calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2].0.get("model"), Some(&text("public")));
    }
    assert_eq!(routing.team_queries.lock().unwrap().len(), 2);
}
