#![cfg(test)]

use crate::schema_transforms::tree::JsonObject;
use crate::websocket_stubs::{QueryParam, WebsocketRoute, inject_websocket_stubs};

fn document(json: &str) -> JsonObject {
    serde_json::from_str(json).expect("the test document is valid JSON")
}

fn route(path: &str, name: Option<&str>, query_params: Vec<QueryParam>) -> WebsocketRoute {
    WebsocketRoute {
        path: path.to_owned(),
        name: name.map(str::to_owned),
        query_params,
    }
}

fn empty_parameters() -> Vec<QueryParam> {
    Vec::new()
}

#[test]
fn a_new_path_gets_a_get_stub_named_after_the_route() {
    let mut openapi = document(r#"{"paths": {}}"#);
    inject_websocket_stubs(
        &mut openapi,
        &[route(
            "/v1/realtime",
            Some("realtime"),
            vec![QueryParam {
                name: "model".to_owned(),
                required: true,
            }],
        )],
    );
    let expected = document(
        r#"{"paths": {"/v1/realtime": {"get": {
            "summary": "WebSocket: realtime",
            "description": "WebSocket connection endpoint",
            "operationId": "websocket_realtime",
            "parameters": [{"name": "model", "in": "query", "required": true, "schema": {"type": "string"}}],
            "responses": {"101": {"description": "WebSocket Protocol Switched"}},
            "tags": ["WebSocket"]
        }}}}"#,
    );
    assert_eq!(openapi, expected);
}

#[test]
fn a_route_without_a_name_is_named_after_its_stub_path() {
    let mut openapi = document(r#"{"paths": {}}"#);
    inject_websocket_stubs(
        &mut openapi,
        &[route("/v1/realtime", None, empty_parameters())],
    );
    let expected = document(
        r#"{"paths": {"/v1/realtime": {"get": {
            "summary": "WebSocket: /v1/realtime",
            "description": "WebSocket connection endpoint",
            "operationId": "websocket__v1_realtime",
            "parameters": [],
            "responses": {"101": {"description": "WebSocket Protocol Switched"}},
            "tags": ["WebSocket"]
        }}}}"#,
    );
    assert_eq!(openapi, expected);
}

#[test]
fn an_existing_get_operation_blocks_the_stub_and_keeps_its_own_content() {
    let mut openapi = document(
        r#"{"paths": {"/v1/realtime": {"get": {"operationId": "real", "summary": "Real"}}}}"#,
    );
    let expected = openapi.clone();
    inject_websocket_stubs(
        &mut openapi,
        &[route("/v1/realtime", Some("realtime"), empty_parameters())],
    );
    assert_eq!(openapi, expected);
}

#[test]
fn a_path_item_with_other_operations_keeps_them_and_gains_the_stub() {
    let mut openapi =
        document(r#"{"paths": {"/v1/realtime": {"post": {"operationId": "real_post"}}}}"#);
    inject_websocket_stubs(
        &mut openapi,
        &[route("/v1/realtime", Some("realtime"), empty_parameters())],
    );
    let expected = document(
        r#"{"paths": {"/v1/realtime": {
            "post": {"operationId": "real_post"},
            "get": {
                "summary": "WebSocket: realtime",
                "description": "WebSocket connection endpoint",
                "operationId": "websocket_realtime",
                "parameters": [],
                "responses": {"101": {"description": "WebSocket Protocol Switched"}},
                "tags": ["WebSocket"]
            }
        }}}"#,
    );
    assert_eq!(openapi, expected);
}

#[test]
fn the_stub_path_is_cut_at_the_first_brace_and_strips_trailing_question_marks() {
    let mut openapi = document(r#"{"paths": {}}"#);
    inject_websocket_stubs(
        &mut openapi,
        &[
            route(
                "/openai/deployments/{model}/realtime",
                None,
                empty_parameters(),
            ),
            route("/rt??", None, empty_parameters()),
        ],
    );
    let expected = document(
        r#"{"paths": {
            "/openai/deployments/": {"get": {
                "summary": "WebSocket: /openai/deployments/",
                "description": "WebSocket connection endpoint",
                "operationId": "websocket__openai_deployments_",
                "parameters": [],
                "responses": {"101": {"description": "WebSocket Protocol Switched"}},
                "tags": ["WebSocket"]
            }},
            "/rt": {"get": {
                "summary": "WebSocket: /rt",
                "description": "WebSocket connection endpoint",
                "operationId": "websocket__rt",
                "parameters": [],
                "responses": {"101": {"description": "WebSocket Protocol Switched"}},
                "tags": ["WebSocket"]
            }}
        }}"#,
    );
    assert_eq!(openapi, expected);
}

#[test]
fn query_parameters_keep_their_order_including_optional_ones() {
    let mut openapi = document(r#"{"paths": {}}"#);
    inject_websocket_stubs(
        &mut openapi,
        &[route(
            "/v1/realtime",
            Some("realtime"),
            vec![
                QueryParam {
                    name: "model".to_owned(),
                    required: true,
                },
                QueryParam {
                    name: "session".to_owned(),
                    required: false,
                },
            ],
        )],
    );
    let expected = document(
        r#"{"paths": {"/v1/realtime": {"get": {
            "summary": "WebSocket: realtime",
            "description": "WebSocket connection endpoint",
            "operationId": "websocket_realtime",
            "parameters": [
                {"name": "model", "in": "query", "required": true, "schema": {"type": "string"}},
                {"name": "session", "in": "query", "required": false, "schema": {"type": "string"}}
            ],
            "responses": {"101": {"description": "WebSocket Protocol Switched"}},
            "tags": ["WebSocket"]
        }}}}"#,
    );
    assert_eq!(openapi, expected);
}

#[test]
fn two_routes_on_one_stub_path_add_only_the_first_get() {
    let mut openapi = document(r#"{"paths": {}}"#);
    inject_websocket_stubs(
        &mut openapi,
        &[
            route("/v1/realtime", Some("first"), empty_parameters()),
            route("/v1/realtime", Some("second"), empty_parameters()),
        ],
    );
    let expected = document(
        r#"{"paths": {"/v1/realtime": {"get": {
            "summary": "WebSocket: first",
            "description": "WebSocket connection endpoint",
            "operationId": "websocket_first",
            "parameters": [],
            "responses": {"101": {"description": "WebSocket Protocol Switched"}},
            "tags": ["WebSocket"]
        }}}}"#,
    );
    assert_eq!(openapi, expected);
}
