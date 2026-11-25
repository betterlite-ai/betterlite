use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const CREATED: &str = r#"{"type": "response.created", "response": {"id": "r-early"}}"#;
const PROGRESS: &str = r#"{"type": "response.progress", "response": {"id": "r-early"}}"#;
const FIRST: &str = r#"{"type": "response.completed", "response": {"id": "r1", "output": [{"type": "message", "content": [{"type": "output_text", "text": "A"}]}]}}"#;
const SECOND: &str = r#"{"type": "response.completed", "response": {"id": "r2", "output": [{"type": "message", "content": [{"type": "output_text", "text": "B"}]}]}}"#;
const FRAME: &str = r#"{"type": "response.create", "input": "hello"}"#;
const STORED: &str = r#"{"r1": [
    {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
    {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "A"}]}
]}"#;

#[tokio::test]
async fn only_serializable_chunks_are_forwarded_and_the_first_completion_is_kept() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let chunks = vec![
        support::empty_chunk(),
        support::event(CREATED),
        support::object_chunk(PROGRESS),
        support::broken_chunk(),
        support::event(FIRST),
        support::event(SECOND),
    ];
    let provider = support::ScriptedProvider::new(&shared, vec![support::script(chunks)]);

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let expected = vec![
        support::json(CREATED),
        support::json(PROGRESS),
        support::json(FIRST),
        support::json(SECOND),
    ];
    assert_eq!(support::frames(&shared), expected);
    assert_eq!(
        logs,
        vec![
            "ManagedResponsesWS: failed to serialize chunk: chunk must be a JSON object",
            "ManagedResponsesWS: stored 2 messages for response_id=r1",
        ]
    );
    assert_eq!(support::history(&handler), support::json(STORED));
    assert_eq!(support::trace(&shared), ["provider", "provider-end"]);
}

#[tokio::test]
async fn an_object_chunk_declares_its_completion_before_serialization() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let dump = r#"{"type": "response.progress", "response": {"id": "r-typed", "output": [{"type": "message", "content": [{"type": "output_text", "text": "T"}]}]}}"#;
    let chunk = support::typed_object(dump, "response.completed");
    let provider = support::ScriptedProvider::new(&shared, vec![support::script(vec![chunk])]);

    let (outcome, logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    assert_eq!(support::frames(&shared), vec![support::json(dump)]);
    assert_eq!(
        logs,
        vec!["ManagedResponsesWS: stored 2 messages for response_id=r-typed"]
    );
    let stored = r#"{"r-typed": [
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hello"}]},
        {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "T"}]}
    ]}"#;
    assert_eq!(support::history(&handler), support::json(stored));
}
