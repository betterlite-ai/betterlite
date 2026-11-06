use super::support::{BASE, ResponseKind, Scenario, answer, json, logger, model_completion};
use crate::braintrust_logging::event::CallbackPath;

#[tokio::test]
async fn missing_project_creates_the_litellm_default() {
    let (logger, replay) = logger(
        vec![answer(r#"{"id":"litellm-project"}"#), answer("{}")],
        &[],
    );
    let mut scenario = Scenario::empty();
    scenario.response_kind = ResponseKind::Model;
    scenario.response_body = model_completion();
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests[0].url, format!("{BASE}/project"));
    assert_eq!(requests[0].body, json(r#"{"name":"litellm"}"#));
    assert_eq!(
        requests[1].url,
        format!("{BASE}/project_logs/litellm-project/insert")
    );
}

#[tokio::test]
async fn default_project_is_created_once_per_logger_default() {
    let (logger, replay) = logger(
        vec![
            answer(r#"{"id":"litellm-project"}"#),
            answer("{}"),
            answer("{}"),
        ],
        &[],
    );
    let scenario = Scenario::empty();
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].body, json(r#"{"name":"litellm"}"#));
    assert_eq!(
        requests[1].url,
        format!("{BASE}/project_logs/litellm-project/insert")
    );
    assert_eq!(
        requests[2].url,
        format!("{BASE}/project_logs/litellm-project/insert")
    );
}

#[tokio::test]
async fn async_default_project_uses_the_project_endpoint_default() {
    let (logger, replay) = logger(vec![answer(r#"{"id":"async-default"}"#), answer("{}")], &[]);
    logger
        .log_success_event(&Scenario::empty().event(), CallbackPath::Async)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests[0].url, format!("{BASE}/project"));
    assert_eq!(requests[0].body, json(r#"{"name":"litellm"}"#));
    assert_eq!(
        requests[1].url,
        format!("{BASE}/project_logs/async-default/insert")
    );
}

#[tokio::test]
async fn an_empty_project_name_creates_the_default() {
    let (logger, replay) = logger(
        vec![answer(r#"{"id":"litellm-project"}"#), answer("{}")],
        &[],
    );
    let mut scenario = Scenario::empty();
    scenario.metadata = json(r#"{"project_name":""}"#);
    logger
        .log_success_event(&scenario.event(), CallbackPath::Sync)
        .await
        .unwrap();
    let requests = replay.requests();
    assert_eq!(requests[0].body, json(r#"{"name":"litellm"}"#));
}
