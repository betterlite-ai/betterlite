#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;

use super::{
    FakeDispatch, MessagesRoute, call_steps, empty_routing, input, names, object,
    plan_messages_route, text,
};

#[tokio::test]
async fn the_override_is_not_merged_when_the_body_carries_an_api_key() {
    let dispatch = Arc::new(FakeDispatch::default());
    let steps = call_steps(Arc::clone(&dispatch), None, None, false, Some("cli"));
    let data = object(&[
        ("model", text("m")),
        ("api_key", text("k")),
        (
            "router_settings_override",
            JsonValue::Object(object(&[("num_retries", JsonValue::from(2_i64))])),
        ),
    ]);
    steps.call(data, false).await.unwrap();
    let calls = dispatch.calls.lock().unwrap();
    assert!(calls[0].contains_key("router_settings_override"));
    assert!(!calls[0].contains_key("num_retries"));
}

#[test]
fn a_team_public_model_that_is_also_a_model_name_is_not_found() {
    let mut routing = empty_routing();
    routing.model_names = names(&["shared"]);
    routing.team_public = names(&["shared"]);
    let mut input = input(Some("shared"), true);
    input.is_proxy_admin = true;
    assert_eq!(
        plan_messages_route(&input, Some(&routing)),
        MessagesRoute::NotFound
    );
}
