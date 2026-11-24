#![cfg(test)]

use betterlite_openapi::document_builder::build_document;
use betterlite_openapi::schema_transforms::tree::{JsonObject, JsonTree};
use rstest::rstest;

use super::document_inputs;

#[rstest]
#[case(
    0,
    "/v1/chat/completions",
    "chat_completion",
    "chat/completions",
    false
)]
#[case(1, "/chat/completions", "chat_completion", "chat/completions", false)]
#[case(
    2,
    "/engines/{model}/chat/completions",
    "chat_completion",
    "chat/completions",
    false
)]
#[case(
    3,
    "/openai/deployments/{model}/chat/completions",
    "chat_completion",
    "chat/completions",
    true
)]
#[case(4, "/v1/embeddings", "embeddings", "embeddings", false)]
#[case(5, "/embeddings", "embeddings", "embeddings", false)]
#[case(6, "/engines/{model}/embeddings", "embeddings", "embeddings", false)]
#[case(
    7,
    "/openai/deployments/{model}/embeddings",
    "embeddings",
    "embeddings",
    false
)]
#[case(8, "/v1/responses", "responses_api", "responses", false)]
#[case(9, "/responses", "responses_api", "responses", false)]
#[case(10, "/openai/v1/responses", "responses_api", "responses", false)]
fn openapi_inputs_list_the_eleven_post_routes(
    #[case] index: usize,
    #[case] path: &str,
    #[case] operation: &str,
    #[case] category: &str,
    #[case] error_responses: bool,
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
    assert_eq!(route.error_responses, error_responses);
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

#[rstest]
#[case("400")]
#[case("401")]
#[case("403")]
#[case("404")]
#[case("408")]
#[case("422")]
#[case("429")]
#[case("500")]
#[case("503")]
fn openapi_document_attaches_the_error_responses_to_the_deployments_route(#[case] status: &str) {
    let document = build_document(&document_inputs("", "1.2.3"));
    let Some(JsonTree::Object(paths)) = document.get("paths") else {
        panic!("the document has paths");
    };
    let Some(JsonTree::Object(deployments)) =
        paths.get("/openai/deployments/{model}/chat/completions")
    else {
        panic!("the document has the deployments chat path");
    };
    let Some(JsonTree::Object(operation)) = deployments.get("post") else {
        panic!("the deployments chat path has a post operation");
    };
    let Some(JsonTree::Object(entries)) = operation.get("responses") else {
        panic!("the deployments operation has responses");
    };
    let Some(JsonTree::Object(response)) = entries.get(status) else {
        panic!("the deployments route has the {status} response");
    };
    let Some(JsonTree::Object(content)) = response.get("content") else {
        panic!("the {status} response has content");
    };
    let Some(JsonTree::Object(media)) = content.get("application/json") else {
        panic!("the {status} response is JSON");
    };
    let Some(JsonTree::Object(schema)) = media.get("schema") else {
        panic!("the {status} response has a schema");
    };
    assert_eq!(
        schema.get("$ref"),
        Some(&JsonTree::Text(
            "#/components/schemas/ErrorResponse".to_owned()
        ))
    );
}

#[test]
fn openapi_document_keeps_the_error_component_off_the_other_routes() {
    let document = build_document(&document_inputs("", "1.2.3"));
    let Some(JsonTree::Object(paths)) = document.get("paths") else {
        panic!("the document has paths");
    };
    let Some(JsonTree::Object(deployments)) =
        paths.get("/openai/deployments/{model}/chat/completions")
    else {
        panic!("the document has the deployments chat path");
    };
    let Some(JsonTree::Object(with_errors)) = deployments.get("post") else {
        panic!("the deployments chat path has a post operation");
    };
    let Some(JsonTree::Object(all_responses)) = with_errors.get("responses") else {
        panic!("the deployments operation has responses");
    };
    assert_eq!(all_responses.len(), 10);
    let Some(JsonTree::Object(chat)) = paths.get("/v1/chat/completions") else {
        panic!("the document has the plain chat path");
    };
    let Some(JsonTree::Object(bare)) = chat.get("post") else {
        panic!("the plain chat path has a post operation");
    };
    let Some(JsonTree::Object(success_only)) = bare.get("responses") else {
        panic!("the plain operation has responses");
    };
    assert_eq!(success_only.len(), 1);
    let Some(JsonTree::Object(components)) = document.get("components") else {
        panic!("the document has components");
    };
    let Some(JsonTree::Object(schemas)) = components.get("schemas") else {
        panic!("the components have schemas");
    };
    assert!(matches!(
        schemas.get("ErrorResponse"),
        Some(JsonTree::Object(_))
    ));
}
