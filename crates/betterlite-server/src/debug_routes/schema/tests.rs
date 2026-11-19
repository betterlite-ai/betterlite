#![cfg(test)]

use axum::http::StatusCode;
use betterlite_openapi::document_builder::{
    DocumentInputs, RouteDoc, build_document, default_info,
};
use betterlite_openapi::llm_request_bodies::LlmRequestSchemas;
use betterlite_openapi::schema_transforms::tree::{JsonObject, JsonTree};

use crate::debug_routes::schema::documented_routes;
use crate::debug_routes::test_support;

const RUNTIME_ROUTES: [(&str, &str); 8] = [
    ("GET", "/debug/asyncio-tasks"),
    ("GET", "/memory-usage"),
    ("GET", "/memory-usage-in-mem-cache"),
    ("GET", "/memory-usage-in-mem-cache-items"),
    ("GET", "/debug/memory/summary"),
    ("GET", "/debug/memory/details"),
    ("POST", "/debug/memory/gc/configure"),
    ("GET", "/otel-spans"),
];

fn document(routes: Vec<RouteDoc>) -> JsonObject {
    build_document(&DocumentInputs {
        info: default_info("", "1.0.0"),
        routes,
        websockets: Vec::new(),
        llm_schemas: LlmRequestSchemas::default(),
        server_root_path: String::new(),
    })
}

fn paths(document: &JsonObject) -> &JsonObject {
    let Some(JsonTree::Object(paths)) = document.get("paths") else {
        panic!("the document has a paths object");
    };
    paths
}

#[test]
fn no_runtime_path_reaches_the_schema() {
    let documentation = documented_routes();
    assert!(documentation.is_empty());
    let built = document(documentation);
    let listed = paths(&built);
    for (_, path) in RUNTIME_ROUTES {
        assert!(!listed.contains_key(path), "{path} stays out of the schema");
    }
}

#[test]
fn the_builder_lists_a_route_it_is_given() {
    let route = RouteDoc {
        path: "/debug/asyncio-tasks".to_owned(),
        method: "GET".to_owned(),
        name: "get_active_tasks_stats".to_owned(),
        summary: None,
        tags: Vec::new(),
        parameters: Vec::new(),
        request_body: None,
        error_responses: false,
    };
    let built = document(vec![route]);
    assert!(paths(&built).contains_key("/debug/asyncio-tasks"));
}

#[tokio::test]
async fn every_runtime_path_is_registered() {
    let router = test_support::router(test_support::state(), &test_support::environment("true"));
    for (method, path) in RUNTIME_ROUTES {
        let (status, _text) = test_support::call(router.clone(), method, path, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} is registered");
    }
}
