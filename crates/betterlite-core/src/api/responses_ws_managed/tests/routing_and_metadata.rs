use secrecy::SecretString;

use super::support;
use crate::api::responses_ws_history::Connection;
use crate::api::responses_ws_managed::process::process_response_create;

const FRAME: &str = r#"{"type": "response.create", "model": "router-model", "input": "hello", "store": true, "tools": [{"type": "function"}], "tool_choice": "auto", "instructions": "be brief", "metadata": {"tenant": "t1"}}"#;
const COMPLETED: &str =
    r#"{"type": "response.completed", "response": {"id": "r-route", "output": []}}"#;

fn credentialed_connection(shared: &support::Shared) -> Connection<support::ScriptedSink> {
    let mut connection = support::connection(support::MODEL, support::sink(shared));
    connection.litellm_metadata = Some(support::object(r#"{"model_group": "router-model"}"#));
    connection.custom_llm_provider = Some("openai".to_owned());
    connection.api_key = Some(SecretString::from("key-1".to_owned()));
    connection.api_base = Some("https://api.example.test".to_owned());
    connection.timeout = Some(7.5_f64);
    connection
        .kwargs
        .insert("stream".to_owned(), support::json("false"));
    connection
}

#[tokio::test]
async fn the_alias_uses_the_connection_model_and_applies_the_extra_kwargs() {
    let shared = support::shared();
    let mut handler = support::handler(credentialed_connection(&shared));
    let scripts = vec![support::script(vec![support::event(COMPLETED)])];
    let provider = support::ScriptedProvider::new(&shared, scripts);

    let (outcome, _logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let recorded = support::calls(&shared);
    assert_eq!(recorded.len(), 1);
    let call = &recorded[0];
    assert_eq!(call.model, support::MODEL);
    assert_eq!(
        call.kwargs.get("custom_llm_provider"),
        Some(&support::json(r#""openai""#))
    );
    assert_eq!(
        call.kwargs.get("stream"),
        Some(&support::json("false")),
        "the extra kwargs are applied last"
    );
    assert_eq!(
        call.kwargs.get("api_key"),
        Some(&support::json(r#""key-1""#))
    );
    assert_eq!(call.kwargs.get("timeout"), Some(&support::json("7.5")));
}

#[tokio::test]
async fn the_proxy_body_names_the_alias_with_the_ordered_fields() {
    let shared = support::shared();
    let mut handler = support::handler(credentialed_connection(&shared));
    let scripts = vec![support::script(vec![support::event(COMPLETED)])];
    let provider = support::ScriptedProvider::new(&shared, scripts);

    let (outcome, _logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let recorded = support::calls(&shared);
    assert_eq!(recorded.len(), 1);
    let body = support::proxy_body(&recorded[0].kwargs);
    assert_eq!(
        support::keys(&body),
        [
            "input",
            "store",
            "model",
            "tools",
            "tool_choice",
            "instructions",
            "metadata"
        ]
    );
    assert_eq!(body.get("model"), Some(&support::json(r#""router-model""#)));
    assert_eq!(body.get("input"), Some(&support::json(r#""hello""#)));
    assert_eq!(body.get("store"), Some(&support::json("true")));
}

#[tokio::test]
async fn an_empty_model_or_group_names_the_resolved_model_in_the_proxy_body() {
    let shared = support::shared();
    let mut handler = support::handler(credentialed_connection(&shared));
    let scripts = vec![
        support::script(vec![support::event(COMPLETED)]),
        support::script(vec![support::event(COMPLETED)]),
    ];
    let provider = support::ScriptedProvider::new(&shared, scripts);
    let empty_model = r#"{"type": "response.create", "model": "", "input": "hello"}"#;
    let (outcome, _logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        empty_model,
    ))
    .await;
    outcome.unwrap();
    handler.model_group = Some(String::new());
    let frame = r#"{"type": "response.create", "input": "hello"}"#;
    let (outcome, _logs) =
        support::capture(process_response_create(&mut handler, &provider, frame)).await;
    outcome.unwrap();
    let recorded = support::calls(&shared);
    assert_eq!(recorded.len(), 2);
    assert!(recorded[0].model.is_empty());
    assert_eq!(
        support::proxy_body(&recorded[0].kwargs).get("model"),
        Some(&support::json(r#""router-model""#))
    );
    assert_eq!(recorded[1].model, support::MODEL);
    assert_eq!(
        support::proxy_body(&recorded[1].kwargs).get("model"),
        Some(&support::json(r#""openai/gpt-4o-mini""#))
    );
}

#[tokio::test]
async fn a_frame_without_a_model_names_the_group_in_the_proxy_body() {
    let shared = support::shared();
    let mut connection = support::connection(support::MODEL, support::sink(&shared));
    connection.litellm_metadata = Some(support::object(r#"{"model_group": "router-model"}"#));
    let mut handler = support::handler(connection);
    let scripts = vec![support::script(vec![support::event(COMPLETED)])];
    let provider = support::ScriptedProvider::new(&shared, scripts);
    let frame = r#"{"type": "response.create", "input": "hello"}"#;

    let (outcome, _logs) =
        support::capture(process_response_create(&mut handler, &provider, frame)).await;
    outcome.unwrap();
    let recorded = support::calls(&shared);
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].model, support::MODEL);
    let body = support::proxy_body(&recorded[0].kwargs);
    assert_eq!(body.get("model"), Some(&support::json(r#""router-model""#)));
}
