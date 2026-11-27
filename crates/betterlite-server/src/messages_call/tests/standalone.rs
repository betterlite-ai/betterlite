#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::chat::execution::ChatExecutor;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rand::SeedableRng as _;
use rand::rngs::StdRng;

use super::{FakeDispatch, MessagesRoute, input, object, plan_messages_route, router_plan, text};
use crate::messages_call::outcome::MessagesDispatch;
use crate::messages_call::standalone::standalone_routing;
use crate::server::chat_setup::chat_executor;

fn dispatch_port(dispatch: Arc<FakeDispatch>) -> Arc<dyn MessagesDispatch> {
    dispatch
}

fn config_executor(yaml: &str) -> Arc<ChatExecutor> {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    Arc::new(chat_executor(&config, &BTreeMap::new(), StdRng::seed_from_u64(7)).unwrap())
}

fn standalone_executor(params: &str) -> Arc<ChatExecutor> {
    let yaml =
        format!("model_list:\n  - model_name: messages-model\n    litellm_params: {params}\n");
    config_executor(&yaml)
}

const DEPLOYMENT: &str =
    "{model: openai/messages-model, api_base: 'http://127.0.0.1:1', api_key: sk-test, timeout: 30}";

#[test]
fn standalone_routing_knows_the_config_groups_and_deployment_names() {
    let routing = standalone_routing(
        "anthropic_messages",
        standalone_executor(DEPLOYMENT),
        Arc::new(FakeDispatch::default()),
    );
    assert!(routing.is_recognized_model("messages-model"));
    assert!(routing.model_names_has("messages-model"));
    assert!(!routing.model_names_has("openai/messages-model"));
    assert!(routing.is_deployment_name("openai/messages-model"));
    assert!(!routing.is_deployment_name("messages-model"));
    assert!(!routing.all_blocked("messages-model", Some("team")));
    assert_eq!(routing.map_team_model(Some("messages-model"), "team"), None);
    assert!(!routing.pass_through_all_models());
    assert!(!routing.has_default_or_pattern());
    assert!(!routing.team_public_model_names_has("messages-model"));
    assert!(routing.deployments_for("messages-model").is_empty());
}

#[test]
fn standalone_routing_plans_its_groups_names_and_unknown_models() {
    let routing = standalone_routing(
        "anthropic_messages",
        standalone_executor(DEPLOYMENT),
        Arc::new(FakeDispatch::default()),
    );
    assert_eq!(
        plan_messages_route(&input(Some("messages-model"), true), Some(routing.as_ref())),
        router_plan(false)
    );
    assert_eq!(
        plan_messages_route(
            &input(Some("openai/messages-model"), true),
            Some(routing.as_ref())
        ),
        router_plan(true)
    );
    assert_eq!(
        plan_messages_route(&input(Some("mystery"), true), Some(routing.as_ref())),
        MessagesRoute::NotFound
    );
}

#[tokio::test]
async fn standalone_anthropic_messages_writes_the_picked_deployment() {
    let executor = standalone_executor(DEPLOYMENT);
    let dispatch = Arc::new(FakeDispatch::default());
    let routing = standalone_routing(
        "anthropic_messages",
        Arc::clone(&executor),
        dispatch_port(Arc::clone(&dispatch)),
    );
    let data = object(&[
        ("model", text("messages-model")),
        (
            "litellm_metadata",
            JsonValue::Object(object(&[
                (
                    "model_info",
                    JsonValue::Object(object(&[("id", text("old"))])),
                ),
                ("other", text("keep")),
            ])),
        ),
    ]);
    let mut outcome = routing.anthropic_messages(data, false).await.unwrap();
    let deployment = &executor.deployments[0];
    let sent = dispatch.calls.lock().unwrap()[0].clone();
    assert_eq!(sent.get("model"), Some(&text("openai/messages-model")));
    assert_eq!(sent.get("api_base"), Some(&text("http://127.0.0.1:1")));
    assert_eq!(sent.get("api_key"), Some(&text("sk-test")));
    assert_eq!(sent.get("timeout"), Some(&JsonValue::from(30.0_f64)));
    let metadata = sent.get("litellm_metadata").unwrap();
    assert_eq!(metadata.get("other"), Some(&text("keep")));
    assert_eq!(
        metadata.get("model_info").and_then(|info| info.get("id")),
        Some(&text(&deployment.model_id))
    );
    let hidden = outcome.hidden_mut();
    assert_eq!(hidden.get("model_id"), Some(&text(&deployment.model_id)));
    assert_eq!(hidden.get("api_base"), Some(&text("http://127.0.0.1:1")));
}

#[tokio::test]
async fn standalone_anthropic_messages_replaces_a_non_object_model_info() {
    let executor = standalone_executor(DEPLOYMENT);
    let dispatch = Arc::new(FakeDispatch::default());
    let routing = standalone_routing(
        "anthropic_messages",
        Arc::clone(&executor),
        dispatch_port(Arc::clone(&dispatch)),
    );
    let numeric = object(&[
        ("model", text("messages-model")),
        (
            "litellm_metadata",
            JsonValue::Object(object(&[("model_info", JsonValue::from(7_i64))])),
        ),
    ]);
    routing.anthropic_messages(numeric, false).await.unwrap();
    let sent = dispatch.calls.lock().unwrap()[0].clone();
    assert_eq!(
        sent.get("litellm_metadata")
            .and_then(|fields| fields.get("model_info"))
            .and_then(|info| info.get("id")),
        Some(&text(&executor.deployments[0].model_id))
    );
}

#[tokio::test]
async fn standalone_specific_deployment_picks_the_named_litellm_model() {
    let executor = standalone_executor(DEPLOYMENT);
    let dispatch = Arc::new(FakeDispatch::default());
    let routing = standalone_routing(
        "anthropic_messages",
        Arc::clone(&executor),
        dispatch_port(Arc::clone(&dispatch)),
    );
    routing
        .anthropic_messages(object(&[("model", text("openai/messages-model"))]), true)
        .await
        .unwrap();
    let sent = dispatch.calls.lock().unwrap()[0].clone();
    assert_eq!(sent.get("model"), Some(&text("openai/messages-model")));
}

const TWO_DEPLOYMENTS: &str = "model_list:\n  - model_name: first\n    litellm_params: {model: openai/first-model, api_base: 'http://127.0.0.1:1', api_key: sk-first}\n  - model_name: second\n    litellm_params: {model: openai/second-model, api_base: 'http://127.0.0.1:2', api_key: sk-second}\n";

#[tokio::test]
async fn standalone_specific_deployment_skips_the_other_litellm_models() {
    let executor = config_executor(TWO_DEPLOYMENTS);
    let dispatch = Arc::new(FakeDispatch::default());
    let routing = standalone_routing(
        "anthropic_messages",
        Arc::clone(&executor),
        dispatch_port(Arc::clone(&dispatch)),
    );
    routing
        .anthropic_messages(object(&[("model", text("openai/second-model"))]), true)
        .await
        .unwrap();
    let sent = dispatch.calls.lock().unwrap()[0].clone();
    assert_eq!(sent.get("model"), Some(&text("openai/second-model")));
    assert_eq!(sent.get("api_base"), Some(&text("http://127.0.0.1:2")));
    assert_eq!(sent.get("api_key"), Some(&text("sk-second")));
}

#[tokio::test]
async fn standalone_a2a_and_an_unknown_pick_answer_no_agent_and_not_found() {
    let executor = standalone_executor(DEPLOYMENT);
    let dispatch = Arc::new(FakeDispatch::default());
    let routing = standalone_routing(
        "anthropic_messages",
        Arc::clone(&executor),
        dispatch_port(Arc::clone(&dispatch)),
    );
    assert!(routing.a2a_agent(JsonMap::new()).await.is_none());
    let Err(error) = routing
        .anthropic_messages(object(&[("model", text("mystery"))]), false)
        .await
    else {
        panic!("an unknown model cannot be picked");
    };
    assert_eq!(error.status(), 400);
}
