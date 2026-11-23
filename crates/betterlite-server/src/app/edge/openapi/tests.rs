#![cfg(test)]

use betterlite_openapi::document_builder::build_document;
use betterlite_openapi::schema_transforms::tree::{JsonObject, JsonTree};
use rstest::rstest;

use super::document_inputs;

#[rstest]
#[case(0, "/v1/chat/completions", "chat_completion", "chat/completions")]
#[case(1, "/chat/completions", "chat_completion", "chat/completions")]
#[case(
    2,
    "/engines/{model}/chat/completions",
    "chat_completion",
    "chat/completions"
)]
#[case(
    3,
    "/openai/deployments/{model}/chat/completions",
    "chat_completion",
    "chat/completions"
)]
#[case(4, "/v1/embeddings", "embeddings", "embeddings")]
#[case(5, "/embeddings", "embeddings", "embeddings")]
#[case(6, "/engines/{model}/embeddings", "embeddings", "embeddings")]
#[case(
    7,
    "/openai/deployments/{model}/embeddings",
    "embeddings",
    "embeddings"
)]
#[case(8, "/v1/responses", "responses_api", "responses")]
#[case(9, "/responses", "responses_api", "responses")]
#[case(10, "/openai/v1/responses", "responses_api", "responses")]
fn openapi_inputs_list_the_eleven_post_routes(
    #[case] index: usize,
    #[case] path: &str,
    #[case] operation: &str,
    #[case] category: &str,
) {
    let inputs = document_inputs("", "1.2.3");
    assert_eq!(inputs.routes.len(), 11);
    let route = &inputs.routes[index];
    assert_eq!(route.path, path);
    assert_eq!(route.method, "POST");
    assert_eq!(route.name, operation);
    assert_eq!(route.tags, vec![category.to_owned()]);
    assert!(route.parameters.is_empty());
    assert!(route.request_body.is_none());
    assert!(route.summary.is_none());
    assert!(!route.error_responses);
}

#[test]
fn openapi_inputs_leave_the_request_schemas_empty() {
    let inputs = document_inputs("", "1.2.3");
    assert!(inputs.llm_schemas.chat_completion.is_none());
    assert!(inputs.llm_schemas.embedding.is_none());
    assert!(inputs.llm_schemas.responses.is_none());
}

#[rstest]
#[case(
    0,
    "/openai/v1/realtime",
    "realtime_websocket_endpoint",
    &["model", "intent", "guardrails"]
)]
#[case(
    1,
    "/v1/realtime",
    "realtime_websocket_endpoint",
    &["model", "intent", "guardrails"]
)]
#[case(
    2,
    "/realtime",
    "realtime_websocket_endpoint",
    &["model", "intent", "guardrails"]
)]
#[case(3, "/v1/responses", "responses_websocket_endpoint", &["model"])]
#[case(4, "/responses", "responses_websocket_endpoint", &["model"])]
fn openapi_inputs_list_the_websocket_routes(
    #[case] index: usize,
    #[case] path: &str,
    #[case] operation: &str,
    #[case] query_params: &[&str],
) {
    let inputs = document_inputs("", "1.2.3");
    assert_eq!(inputs.websockets.len(), 5);
    let route = &inputs.websockets[index];
    assert_eq!(route.path, path);
    assert_eq!(route.name.as_deref(), Some(operation));
    let declared: Vec<&str> = route
        .query_params
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    assert_eq!(declared.as_slice(), query_params);
    assert!(route.query_params.iter().all(|param| !param.required));
}

#[test]
fn openapi_document_uses_the_given_version_and_title() {
    let document = build_document(&document_inputs("/api", "1.2.3"));
    let Some(JsonTree::Object(info)) = document.get("info") else {
        panic!("the document has info");
    };
    assert_eq!(
        info.get("title"),
        Some(&JsonTree::Text("LiteLLM API".to_owned()))
    );
    assert_eq!(
        info.get("version"),
        Some(&JsonTree::Text("1.2.3".to_owned()))
    );
}

#[test]
fn openapi_document_adds_the_websocket_operations() {
    let document = build_document(&document_inputs("", "1.2.3"));
    let Some(JsonTree::Object(paths)) = document.get("paths") else {
        panic!("the document has paths");
    };
    assert_eq!(paths.len(), 14);
    let Some(JsonTree::Object(realtime)) = paths.get("/realtime") else {
        panic!("the document has the realtime path");
    };
    let Some(JsonTree::Object(stub)) = realtime.get("get") else {
        panic!("the realtime path has a get operation");
    };
    assert_eq!(
        stub.get("summary"),
        Some(&JsonTree::Text(
            "WebSocket: realtime_websocket_endpoint".to_owned()
        ))
    );
    let Some(JsonTree::Object(responses)) = paths.get("/responses") else {
        panic!("the document has the responses path");
    };
    let Some(JsonTree::Object(operation)) = responses.get("post") else {
        panic!("the responses path has the post operation");
    };
    assert_eq!(
        operation.get("operationId"),
        Some(&JsonTree::Text("responses_api_responses_post".to_owned()))
    );
}

#[rstest]
#[case("")]
#[case("/api")]
#[case("/api/")]
fn openapi_document_carries_the_server_entry_only_for_a_non_empty_root_path(
    #[case] server_root_path: &str,
) {
    let inputs = document_inputs(server_root_path, "1.2.3");
    assert_eq!(inputs.server_root_path, server_root_path);
    let document = build_document(&inputs);
    let entries = document.get("servers");
    if server_root_path.is_empty() {
        assert!(entries.is_none());
        return;
    }
    let expected = JsonTree::Array(vec![JsonTree::Object(JsonObject::from([(
        "url".to_owned(),
        JsonTree::Text("/api".to_owned()),
    )]))]);
    assert_eq!(entries, Some(&expected));
}
