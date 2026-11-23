#![cfg(test)]

use betterlite_openapi::document_builder::build_document;
use betterlite_openapi::schema_transforms::tree::{JsonObject, JsonTree};
use rstest::rstest;

use super::document_inputs;

const EXPECTED_POST_ROUTES: [(&str, &str, &str); 11] = [
    (
        "/v1/chat/completions",
        "chat_completion",
        "chat/completions",
    ),
    ("/chat/completions", "chat_completion", "chat/completions"),
    (
        "/engines/{model}/chat/completions",
        "chat_completion",
        "chat/completions",
    ),
    (
        "/openai/deployments/{model}/chat/completions",
        "chat_completion",
        "chat/completions",
    ),
    ("/v1/embeddings", "embeddings", "embeddings"),
    ("/embeddings", "embeddings", "embeddings"),
    ("/engines/{model}/embeddings", "embeddings", "embeddings"),
    (
        "/openai/deployments/{model}/embeddings",
        "embeddings",
        "embeddings",
    ),
    ("/v1/responses", "responses_api", "responses"),
    ("/responses", "responses_api", "responses"),
    ("/openai/v1/responses", "responses_api", "responses"),
];

const EXPECTED_WEBSOCKET_ROUTES: [(&str, &str, &[&str]); 5] = [
    (
        "/openai/v1/realtime",
        "realtime_websocket_endpoint",
        &["model", "intent", "guardrails"],
    ),
    (
        "/v1/realtime",
        "realtime_websocket_endpoint",
        &["model", "intent", "guardrails"],
    ),
    (
        "/realtime",
        "realtime_websocket_endpoint",
        &["model", "intent", "guardrails"],
    ),
    ("/v1/responses", "responses_websocket_endpoint", &["model"]),
    ("/responses", "responses_websocket_endpoint", &["model"]),
];

#[test]
fn openapi_inputs_list_the_eleven_post_routes() {
    let inputs = document_inputs("", "1.2.3");
    assert_eq!(inputs.routes.len(), EXPECTED_POST_ROUTES.len());
    for (route, (path, name, tag)) in inputs.routes.iter().zip(EXPECTED_POST_ROUTES) {
        assert_eq!(route.path, path);
        assert_eq!(route.method, "POST");
        assert_eq!(route.name, name);
        assert_eq!(route.tags, vec![tag.to_owned()]);
        assert!(route.parameters.is_empty());
        assert!(route.request_body.is_none());
        assert!(route.summary.is_none());
        assert!(!route.error_responses);
    }
    assert!(inputs.llm_schemas.chat_completion.is_none());
    assert!(inputs.llm_schemas.embedding.is_none());
    assert!(inputs.llm_schemas.responses.is_none());
}

#[test]
fn openapi_inputs_list_the_websocket_routes() {
    let inputs = document_inputs("", "1.2.3");
    assert_eq!(inputs.websockets.len(), EXPECTED_WEBSOCKET_ROUTES.len());
    for (route, (path, name, query_params)) in
        inputs.websockets.iter().zip(EXPECTED_WEBSOCKET_ROUTES)
    {
        assert_eq!(route.path, path);
        assert_eq!(route.name.as_deref(), Some(name));
        assert_eq!(route.query_params.len(), query_params.len());
        for (param, expected) in route.query_params.iter().zip(query_params) {
            assert_eq!(param.name, *expected);
            assert!(!param.required);
        }
    }
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
    let servers = document.get("servers");
    if server_root_path.is_empty() {
        assert!(servers.is_none());
        return;
    }
    let expected = JsonTree::Array(vec![JsonTree::Object(JsonObject::from([(
        "url".to_owned(),
        JsonTree::Text("/api".to_owned()),
    )]))]);
    assert_eq!(servers, Some(&expected));
}
