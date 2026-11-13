#![cfg(test)]

use crate::llm_request_bodies::{
    CHAT_COMPLETION_PATHS, EMBEDDING_PATHS, LlmRequestSchemas, RESPONSES_API_PATHS,
    add_llm_request_bodies,
};
use crate::schema_transforms::tree::{JsonObject, JsonTree};

const ALL_PATHS: [&str; 11] = [
    "/v1/chat/completions",
    "/chat/completions",
    "/engines/{model}/chat/completions",
    "/openai/deployments/{model}/chat/completions",
    "/v1/embeddings",
    "/embeddings",
    "/engines/{model}/embeddings",
    "/openai/deployments/{model}/embeddings",
    "/v1/responses",
    "/responses",
    "/openai/v1/responses",
];

const CHAT_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {"model": {"type": "string"}, "messages": {"type": "array", "items": {"type": "object"}}},
    "required": ["model", "messages"]
}"#;

const CHAT_REQUEST_BODY: &str = r#"{
    "required": true,
    "content": {"application/json": {"schema": {
        "type": "object",
        "required": ["model", "messages"],
        "properties": {
            "model": {"type": "string"},
            "messages": {
                "type": "array",
                "items": {"type": "object"},
                "example": [{"role": "user", "content": "Hello, how are you?"}]
            }
        }
    }}}
}"#;

const EMBEDDING_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {"model": {"type": "string"}, "input": {}},
    "required": ["model", "input"]
}"#;

const EMBEDDING_REQUEST_BODY: &str = r#"{
    "required": true,
    "content": {"application/json": {"schema": {
        "type": "object",
        "required": ["model", "input"],
        "properties": {"model": {"type": "string"}, "input": {}}
    }}}
}"#;

const RESPONSES_SCHEMA: &str = r#"{
    "type": "object",
    "properties": {"model": {"type": "string"}, "input": {"type": "string"}},
    "required": ["model", "input"]
}"#;

const RESPONSES_REQUEST_BODY: &str = r#"{
    "required": true,
    "content": {"application/json": {"schema": {
        "type": "object",
        "required": ["model", "input"],
        "properties": {"model": {"type": "string"}, "input": {"type": "string"}}
    }}}
}"#;

fn document(json: &str) -> JsonObject {
    serde_json::from_str(json).expect("the test document is valid JSON")
}

fn document_with_every_llm_path() -> JsonObject {
    let paths = ALL_PATHS
        .iter()
        .map(|path| {
            (
                (*path).to_owned(),
                JsonTree::Object(JsonObject::from([(
                    "post".to_owned(),
                    JsonTree::Object(JsonObject::from([(
                        "operationId".to_owned(),
                        JsonTree::Text((*path).to_owned()),
                    )])),
                )])),
            )
        })
        .collect::<JsonObject>();
    JsonObject::from([
        ("paths".to_owned(), JsonTree::Object(paths)),
        (
            "components".to_owned(),
            JsonTree::Object(JsonObject::from([(
                "schemas".to_owned(),
                JsonTree::Object(JsonObject::new()),
            )])),
        ),
    ])
}

fn at<'openapi>(openapi: &'openapi JsonObject, keys: &[&str]) -> Option<&'openapi JsonObject> {
    let mut current = openapi;
    for key in keys {
        let JsonTree::Object(next) = current.get(*key)? else {
            return None;
        };
        current = next;
    }
    Some(current)
}

fn request_body<'openapi>(
    openapi: &'openapi JsonObject,
    path: &str,
) -> Option<&'openapi JsonObject> {
    at(openapi, &["paths", path, "post", "requestBody"])
}

fn component<'openapi>(openapi: &'openapi JsonObject, name: &str) -> Option<&'openapi JsonObject> {
    at(openapi, &["components", "schemas", name])
}

fn assert_only_these_paths_carry_the_body(
    openapi: &JsonObject,
    decorated: &[&str],
    expected_body: &str,
) {
    let expected = document(expected_body);
    for path in ALL_PATHS {
        let body = request_body(openapi, path);
        if decorated.contains(&path) {
            assert_eq!(body, Some(&expected), "path {path} carries no body");
        } else {
            assert_eq!(body, None, "path {path} was decorated unexpectedly");
        }
    }
}

#[test]
fn the_chat_completion_schema_decorates_its_four_paths() {
    let mut openapi = document_with_every_llm_path();
    add_llm_request_bodies(
        &mut openapi,
        &LlmRequestSchemas {
            chat_completion: Some(document(CHAT_SCHEMA)),
            embedding: None,
            responses: None,
        },
    );
    assert_eq!(
        component(&openapi, "ProxyChatCompletionRequest"),
        Some(&document(CHAT_SCHEMA))
    );
    assert_eq!(component(&openapi, "EmbeddingRequest"), None);
    assert_eq!(component(&openapi, "ResponsesAPIRequestParams"), None);
    assert_only_these_paths_carry_the_body(&openapi, &CHAT_COMPLETION_PATHS, CHAT_REQUEST_BODY);
}

#[test]
fn the_embedding_schema_decorates_its_four_paths() {
    let mut openapi = document_with_every_llm_path();
    add_llm_request_bodies(
        &mut openapi,
        &LlmRequestSchemas {
            chat_completion: None,
            embedding: Some(document(EMBEDDING_SCHEMA)),
            responses: None,
        },
    );
    assert_eq!(
        component(&openapi, "EmbeddingRequest"),
        Some(&document(EMBEDDING_SCHEMA))
    );
    assert_eq!(component(&openapi, "ProxyChatCompletionRequest"), None);
    assert_eq!(component(&openapi, "ResponsesAPIRequestParams"), None);
    assert_only_these_paths_carry_the_body(&openapi, &EMBEDDING_PATHS, EMBEDDING_REQUEST_BODY);
}

#[test]
fn the_responses_schema_decorates_its_three_paths() {
    let mut openapi = document_with_every_llm_path();
    add_llm_request_bodies(
        &mut openapi,
        &LlmRequestSchemas {
            chat_completion: None,
            embedding: None,
            responses: Some(document(RESPONSES_SCHEMA)),
        },
    );
    assert_eq!(
        component(&openapi, "ResponsesAPIRequestParams"),
        Some(&document(RESPONSES_SCHEMA))
    );
    assert_eq!(component(&openapi, "ProxyChatCompletionRequest"), None);
    assert_eq!(component(&openapi, "EmbeddingRequest"), None);
    assert_only_these_paths_carry_the_body(&openapi, &RESPONSES_API_PATHS, RESPONSES_REQUEST_BODY);
}

#[test]
fn all_three_schemas_register_components_in_chat_embedding_responses_order() {
    let mut openapi = document_with_every_llm_path();
    add_llm_request_bodies(
        &mut openapi,
        &LlmRequestSchemas {
            chat_completion: Some(document(CHAT_SCHEMA)),
            embedding: Some(document(EMBEDDING_SCHEMA)),
            responses: Some(document(RESPONSES_SCHEMA)),
        },
    );
    let JsonTree::Object(components) = openapi.get("components").expect("components exist") else {
        panic!("components is an object");
    };
    let JsonTree::Object(schemas) = components.get("schemas").expect("schemas exist") else {
        panic!("schemas is an object");
    };
    assert_eq!(
        schemas.keys().map(String::as_str).collect::<Vec<&str>>(),
        vec![
            "ProxyChatCompletionRequest",
            "EmbeddingRequest",
            "ResponsesAPIRequestParams"
        ]
    );
    assert_eq!(
        request_body(&openapi, "/v1/chat/completions"),
        Some(&document(CHAT_REQUEST_BODY))
    );
    assert_eq!(
        request_body(&openapi, "/v1/embeddings"),
        Some(&document(EMBEDDING_REQUEST_BODY))
    );
    assert_eq!(
        request_body(&openapi, "/v1/responses"),
        Some(&document(RESPONSES_REQUEST_BODY))
    );
}

#[test]
fn absent_schemas_leave_their_endpoints_and_components_untouched() {
    let mut openapi = document_with_every_llm_path();
    let expected = openapi.clone();
    add_llm_request_bodies(&mut openapi, &LlmRequestSchemas::default());
    assert_eq!(openapi, expected);
}
