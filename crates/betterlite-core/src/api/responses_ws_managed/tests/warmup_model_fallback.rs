use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const SESSION_FRAME: &str = r#"{"type": "response.create", "response": {"generate": false}}"#;

#[tokio::test]
async fn a_model_group_names_both_acknowledgements() {
    let shared = support::shared();
    let mut connection = support::connection(support::MODEL, support::sink(&shared));
    connection.litellm_metadata = Some(support::object(r#"{"model_group": "router-model"}"#));
    let mut handler = support::handler(connection);
    let provider = support::ScriptedProvider::idle(&shared);

    let (outcome, logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        SESSION_FRAME,
    ))
    .await;
    outcome.unwrap();
    assert_eq!(
        support::frames(&shared),
        vec![
            support::warmup_ack("response.created", "in_progress", support::GROUP),
            support::warmup_ack("response.completed", "completed", support::GROUP),
        ]
    );
    assert!(logs.is_empty());
}

#[tokio::test]
async fn an_absent_model_group_falls_back_to_the_connection_model() {
    let shared = support::shared();
    let mut handler = support::handler(support::connection(support::MODEL, support::sink(&shared)));
    let provider = support::ScriptedProvider::idle(&shared);

    let (outcome, logs) = support::capture(process_response_create(
        &mut handler,
        &provider,
        SESSION_FRAME,
    ))
    .await;
    outcome.unwrap();
    assert_eq!(
        support::frames(&shared),
        vec![
            support::warmup_ack("response.created", "in_progress", support::MODEL),
            support::warmup_ack("response.completed", "completed", support::MODEL),
        ]
    );
    assert!(support::calls(&shared).is_empty());
    assert_eq!(support::history(&handler), support::json("{}"));
    assert!(logs.is_empty());
}
