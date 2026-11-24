use secrecy::SecretString;

use super::support;
use crate::api::responses_ws_managed::process::process_response_create;

const FRAME: &str =
    r#"{"type": "response.create", "model": "openai/gpt-4o-mini", "input": "hello"}"#;
const COMPLETED: &str =
    r#"{"type": "response.completed", "response": {"id": "r-other", "output": []}}"#;

#[tokio::test]
async fn a_different_provider_model_keeps_its_name_and_drops_the_connection_provider() {
    let shared = support::shared();
    let mut connection = support::connection("vertex_ai/gemini-2.0-flash", support::sink(&shared));
    connection.litellm_metadata = Some(support::object(r#"{"model_group": "router-model"}"#));
    connection.custom_llm_provider = Some("vertex_ai".to_owned());
    connection.api_key = Some(SecretString::from("key-vertex".to_owned()));
    let mut handler = support::handler(connection);
    let scripts = vec![support::script(vec![support::event(COMPLETED)])];
    let provider = support::ScriptedProvider::new(&shared, scripts);

    let (outcome, _logs) =
        support::capture(process_response_create(&mut handler, &provider, FRAME)).await;
    outcome.unwrap();
    let recorded = support::calls(&shared);
    assert_eq!(recorded.len(), 1);
    let call = &recorded[0];
    assert_eq!(call.model, "openai/gpt-4o-mini");
    assert!(!call.kwargs.contains_key("custom_llm_provider"));
    assert_eq!(
        call.kwargs.get("api_key"),
        Some(&support::json(r#""key-vertex""#))
    );
    let body = support::proxy_body(&call.kwargs);
    assert_eq!(
        body.get("model"),
        Some(&support::json(r#""openai/gpt-4o-mini""#))
    );
}
