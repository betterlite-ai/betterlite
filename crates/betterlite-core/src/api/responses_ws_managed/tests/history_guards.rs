use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const FRAME: &str = r#"{"type": "response.create", "input": "hello"}"#;
const CREATED: &str = r#"{"type": "response.created", "response": {"id": "r-part"}}"#;
const NO_ID: &str = r#"{"type": "response.completed", "response": {"output": []}}"#;
const NUMERIC_ID: &str = r#"{"type": "response.completed", "response": {"id": 7, "output": []}}"#;
const EMPTY_ID: &str = r#"{"type": "response.completed", "response": {"id": "", "output": []}}"#;
const NULL_ID: &str = r#"{"type": "response.completed", "response": {"id": null, "output": []}}"#;

#[tokio::test]
async fn a_stream_without_a_completion_writes_no_history() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let provider = support::ScriptedProvider::new(
        &shared,
        vec![support::script(vec![support::event(CREATED)])],
    );

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    assert_eq!(support::frames(&shared), vec![support::json(CREATED)]);
    assert_eq!(support::history(&handler), support::json("{}"));
    assert!(logs.is_empty());
}

#[tokio::test]
async fn a_completion_without_a_usable_id_writes_no_history() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let chunks = vec![
        support::event(NO_ID),
        support::event(NUMERIC_ID),
        support::event(EMPTY_ID),
        support::event(NULL_ID),
    ];
    let provider = support::ScriptedProvider::new(&shared, vec![support::script(chunks)]);

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let expected = vec![
        support::json(NO_ID),
        support::json(NUMERIC_ID),
        support::json(EMPTY_ID),
        support::json(NULL_ID),
    ];
    assert_eq!(support::frames(&shared), expected);
    assert_eq!(support::history(&handler), support::json("{}"));
    assert!(logs.is_empty());
}

#[tokio::test]
async fn a_completion_that_serializes_to_a_non_object_writes_no_history() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let list = support::typed_object("[]", "response.completed");
    let text = support::typed_object(r#""text""#, "response.completed");
    let provider = support::ScriptedProvider::new(&shared, vec![support::script(vec![list, text])]);

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    assert_eq!(
        support::frames(&shared),
        vec![support::json("[]"), support::json(r#""text""#)]
    );
    assert_eq!(support::history(&handler), support::json("{}"));
    assert!(logs.is_empty());
}
