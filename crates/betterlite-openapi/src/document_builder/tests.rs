#![cfg(test)]

use rstest::rstest;

use crate::document_builder::{
    DocumentInputs, OpenApiInfo, ParamDoc, RouteDoc, build_document, default_info,
};
use crate::error_responses::{error_response_schema, error_responses};
use crate::llm_request_bodies::LlmRequestSchemas;
use crate::schema_transforms::tree::{JsonObject, JsonTree};
use crate::websocket_stubs::{QueryParam, WebsocketRoute};

fn document(json: &str) -> JsonObject {
    serde_json::from_str(json).expect("the test document is valid JSON")
}

fn info() -> OpenApiInfo {
    OpenApiInfo {
        title: "T".to_owned(),
        version: "1".to_owned(),
        description: "D".to_owned(),
    }
}

fn route(path: &str, method: &str, name: &str) -> RouteDoc {
    RouteDoc {
        path: path.to_owned(),
        method: method.to_owned(),
        name: name.to_owned(),
        summary: None,
        tags: Vec::new(),
        parameters: Vec::new(),
        request_body: None,
        error_responses: false,
    }
}

fn inputs(routes: Vec<RouteDoc>, server_root_path: &str) -> DocumentInputs {
    DocumentInputs {
        info: info(),
        routes,
        websockets: Vec::new(),
        llm_schemas: LlmRequestSchemas::default(),
        server_root_path: server_root_path.to_owned(),
    }
}

fn object(tree: &JsonTree) -> &JsonObject {
    let JsonTree::Object(object) = tree else {
        panic!("expected an object");
    };
    object
}

fn operation<'document>(
    document: &'document JsonObject,
    path: &str,
    method: &str,
) -> &'document JsonObject {
    let paths = object(document.get("paths").expect("the document has paths"));
    let path_item = object(paths.get(path).expect("the path exists"));
    object(path_item.get(method).expect("the method exists"))
}

fn responses_of<'document>(document: &'document JsonObject, path: &str) -> &'document JsonObject {
    object(
        operation(document, path, "post")
            .get("responses")
            .expect("the operation has responses"),
    )
}

fn schemas(document: &JsonObject) -> &JsonObject {
    let components = object(
        document
            .get("components")
            .expect("the document has components"),
    );
    object(components.get("schemas").expect("components have schemas"))
}

#[test]
fn build_document_sets_openapi_3_1_and_the_given_info() {
    let built = build_document(&inputs(Vec::new(), ""));
    let expected = document(
        r#"{"openapi": "3.1.0", "info": {"title": "T", "version": "1", "description": "D"},
            "paths": {}, "components": {"schemas": {}}}"#,
    );
    assert_eq!(built, expected);
}

#[rstest]
#[case::the_root_gives_a_slash("/", Some("/"))]
#[case::a_prefix_keeps_its_leading_slash("/api", Some("/api"))]
#[case::surrounding_slashes_are_stripped("/api/", Some("/api"))]
#[case::a_bare_prefix_gains_a_slash("api", Some("/api"))]
#[case::an_empty_root_adds_no_servers("", None)]
fn servers_are_set_only_for_a_non_empty_root_path(
    #[case] root: &str,
    #[case] expected_url: Option<&str>,
) {
    let built = build_document(&inputs(Vec::new(), root));
    let servers = built.get("servers");
    match expected_url {
        Some(url) => assert_eq!(
            servers,
            Some(&JsonTree::Array(vec![JsonTree::Object(JsonObject::from(
                [("url".to_owned(), JsonTree::Text(url.to_owned()),)]
            ))]))
        ),
        None => assert_eq!(servers, None),
    }
}

fn expected_response_keys() -> Vec<String> {
    let mut keys = vec!["200".to_owned()];
    keys.extend(
        error_responses()
            .iter()
            .map(|error| error.status.to_string()),
    );
    keys
}

fn assert_error_statuses(responses: &JsonObject) {
    for error in error_responses() {
        let response = object(
            responses
                .get(&error.status.to_string())
                .expect("the error status has a response"),
        );
        assert_eq!(
            response.get("description"),
            Some(&JsonTree::Text(error.description.clone()))
        );
        let content = object(response.get("content").expect("the response has content"));
        let media = object(
            content
                .get("application/json")
                .expect("the response is JSON"),
        );
        assert_eq!(
            media.get("schema"),
            Some(&JsonTree::Object(JsonObject::from([(
                "$ref".to_owned(),
                JsonTree::Text("#/components/schemas/ErrorResponse".to_owned()),
            )])))
        );
    }
}

#[test]
fn error_responses_and_the_component_are_attached_only_to_routes_that_ask() {
    let mut decorated = route(
        "/openai/deployments/{model}/chat/completions",
        "post",
        "chat_completion",
    );
    decorated.error_responses = true;
    let built = build_document(&inputs(
        vec![
            decorated,
            route("/v1/chat/completions", "post", "chat_completion"),
        ],
        "",
    ));
    let decorated_responses = responses_of(&built, "/openai/deployments/{model}/chat/completions");
    assert_eq!(
        decorated_responses.keys().cloned().collect::<Vec<String>>(),
        expected_response_keys()
    );
    assert_error_statuses(decorated_responses);
    assert_eq!(
        schemas(&built).get("ErrorResponse"),
        Some(&JsonTree::Object(error_response_schema()))
    );
    let plain_responses = responses_of(&built, "/v1/chat/completions");
    assert_eq!(
        plain_responses.keys().cloned().collect::<Vec<String>>(),
        vec!["200".to_owned()]
    );
}

#[test]
fn a_route_becomes_a_fastapi_shaped_operation() {
    let mut route = route(
        "/engines/{model}/chat/completions",
        "POST",
        "chat_completion",
    );
    route.tags = vec!["Chat".to_owned()];
    route.parameters = vec![ParamDoc {
        name: "model".to_owned(),
        location: "path".to_owned(),
        required: true,
        schema: document(r#"{"type": "string"}"#),
    }];
    route.request_body = Some(document(r#"{"required": true, "content": {}}"#));
    let built = build_document(&inputs(vec![route], ""));
    let expected = document(
        r#"{"/engines/{model}/chat/completions": {"post": {
            "tags": ["Chat"],
            "summary": "Chat Completion",
            "operationId": "chat_completion_engines__model__chat_completions_post",
            "parameters": [{"name": "model", "in": "path", "required": true, "schema": {"type": "string"}}],
            "requestBody": {"required": true, "content": {}},
            "responses": {"200": {"description": "Successful Response", "content": {"application/json": {"schema": {}}}}}
        }}}"#,
    );
    assert_eq!(object(built.get("paths").expect("paths")), &expected);
}

#[test]
fn an_operation_omits_empty_tags_and_parameters_and_uses_the_given_summary() {
    let mut route = route("/plain", "get", "plain");
    route.summary = Some("A given summary".to_owned());
    let built = build_document(&inputs(vec![route], ""));
    let expected = document(
        r#"{"summary": "A given summary",
            "operationId": "plain_plain_get",
            "responses": {"200": {"description": "Successful Response", "content": {"application/json": {"schema": {}}}}}}"#,
    );
    assert_eq!(operation(&built, "/plain", "get"), &expected);
}

#[rstest]
#[case::underscores_become_spaces("chat_completion", "Chat Completion")]
#[case::a_digit_starts_a_cased_word("handle_2fa", "Handle 2Fa")]
#[case::a_dash_is_an_uncased_separator("foo-bar", "Foo-Bar")]
#[case::uppercase_inside_a_word_is_lowered("myURL", "Myurl")]
fn derived_summaries_follow_pythons_title_case(#[case] name: &str, #[case] summary: &str) {
    let built = build_document(&inputs(vec![route("/x", "get", name)], ""));
    assert_eq!(
        operation(&built, "/x", "get").get("summary"),
        Some(&JsonTree::Text(summary.to_owned()))
    );
}

fn colliding_websockets() -> Vec<WebsocketRoute> {
    vec![
        WebsocketRoute {
            path: "/a".to_owned(),
            name: Some("shared".to_owned()),
            query_params: vec![QueryParam {
                name: "model".to_owned(),
                required: true,
            }],
        },
        WebsocketRoute {
            path: "/b".to_owned(),
            name: Some("shared".to_owned()),
            query_params: Vec::new(),
        },
    ]
}

fn chat_only_schemas() -> LlmRequestSchemas {
    LlmRequestSchemas {
        chat_completion: Some(document(
            r#"{"type": "object", "properties": {"model": {"type": "string"}}, "required": ["model"]}"#,
        )),
        embedding: None,
        responses: None,
    }
}

#[test]
fn websocket_stubs_and_request_bodies_are_present_before_ids_are_deduplicated() {
    let built = build_document(&DocumentInputs {
        info: info(),
        routes: vec![route("/v1/chat/completions", "post", "chat_completion")],
        websockets: colliding_websockets(),
        llm_schemas: chat_only_schemas(),
        server_root_path: String::new(),
    });
    assert_eq!(
        operation(&built, "/a", "get").get("operationId"),
        Some(&JsonTree::Text("websocket_shared_get".to_owned()))
    );
    assert_eq!(
        operation(&built, "/b", "get").get("operationId"),
        Some(&JsonTree::Text("websocket_shared_get_2".to_owned()))
    );
    assert!(
        operation(&built, "/v1/chat/completions", "post")
            .get("requestBody")
            .is_some()
    );
    assert!(schemas(&built).contains_key("ProxyChatCompletionRequest"));
}

#[test]
fn default_info_carries_the_litellm_title_and_the_three_ui_link_lines() {
    let info = default_info("/api", "9.9.9");
    assert_eq!(info.title, "LiteLLM API");
    assert_eq!(info.version, "9.9.9");
    assert_eq!(
        info.description,
        "Proxy Server to call 100+ LLMs in the OpenAI format. \
[**Customize Swagger Docs**](https://docs.litellm.ai/docs/proxy/enterprise#swagger-docs---custom-routes--branding)\n\n\
\u{1F449} [```LiteLLM Admin Panel on /ui```](/api/ui). Create, Edit Keys with SSO. \
Having issues? Try [```Fallback Login```](/api/fallback/login)\n\n\
\u{1F4B8} [```LiteLLM Model Cost Map```](https://models.litellm.ai/).\n\n\
\u{1F50E} [```LiteLLM Model Hub```](/api/ui/model_hub_table). \
See available models on the proxy. [**Docs**](https://docs.litellm.ai/docs/proxy/ai_hub)"
    );
}

#[test]
fn default_info_with_an_empty_root_path_links_to_the_root() {
    let info = default_info("", "0.1.0");
    assert!(
        info.description
            .contains("[```LiteLLM Admin Panel on /ui```](/ui)")
    );
    assert!(
        info.description
            .contains("[```Fallback Login```](/fallback/login)")
    );
    assert!(
        info.description
            .contains("[```LiteLLM Model Hub```](/ui/model_hub_table)")
    );
}
