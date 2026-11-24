use std::error::Error as _;

use indexmap::IndexMap;

use crate::openapi_tools::document::{OpenApiDocument, parse_openapi_document};

fn parse(text: &str) -> OpenApiDocument {
    parse_openapi_document(text.as_bytes()).unwrap()
}

fn keys<V>(map: &IndexMap<String, V>) -> Vec<&str> {
    map.keys().map(String::as_str).collect()
}

#[test]
fn keeps_paths_and_methods_in_file_order() {
    let document = parse(r#"{"paths":{"/z":{"post":{},"get":{}},"/a":{"delete":{}}}}"#);
    assert_eq!(keys(&document.paths), vec!["/z", "/a"]);
    assert_eq!(keys(&document.paths["/z"].operations), vec!["post", "get"]);
}

#[test]
fn a_path_item_drops_its_non_operation_keys() {
    let document = parse(
        r##"{"paths":{"/pets":{"$ref":"#/x","summary":"pets","x-order":1,"get":{},"post":{}}}}"##,
    );
    assert_eq!(
        keys(&document.paths["/pets"].operations),
        vec!["get", "post"]
    );
}

#[test]
fn null_sections_are_absent() {
    let document = parse(r#"{"servers":null,"schemes":null,"paths":null,"components":null}"#);
    assert!(document.servers.is_empty());
    assert!(document.schemes.is_empty());
    assert!(document.paths.is_empty());
    assert!(document.components.parameters.is_empty());
    assert_eq!(document.host, None);
    assert_eq!(document.base_path, None);
}

#[test]
fn a_method_value_that_is_not_an_object_is_an_error() {
    let error = parse_openapi_document(br#"{"paths":{"/pets":{"get":3}}}"#).unwrap_err();
    assert_eq!(error.to_string(), "the OpenAPI document is not valid JSON");
    let source = error.source().unwrap().to_string();
    assert!(
        source.contains("cannot read its \"get\" operation as an object"),
        "unexpected message: {source}"
    );
}
