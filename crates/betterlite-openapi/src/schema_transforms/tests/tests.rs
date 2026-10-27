#![cfg(test)]

use betterlite_openapi::inline_expansion::{InlineError, inline_request_body};
use betterlite_openapi::schema_transforms::tree::{JsonObject, JsonTree};
use betterlite_openapi::schema_transforms::{
    add_request_body_to_paths, add_request_schema, add_schema_to_components,
};

fn parse(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

const CHAT_SCHEMA: &str = r##"{
    "type":"object","title":"ProxyChatCompletionRequest","required":["model","messages"],
    "properties":{"model":{"type":"string"},"messages":{"type":"array","items":{"$ref":"#/$defs/Message"}}},
    "$defs":{"Message":{"type":"object","properties":{"role":{"type":"string"},"content":{"type":"string"}}}}
}"##;

#[test]
fn chat_request_schema_decorates_document() {
    let mut document = parse(
        r#"{"openapi":"3.1.0","paths":{"/v1/chat/completions":{"post":{"parameters":[{"in":"query","name":"model"}]}},"/chat/completions":{"get":{}}}}"#,
    );
    let paths = ["/v1/chat/completions", "/chat/completions"];
    add_request_schema(
        &mut document,
        "ProxyChatCompletionRequest",
        Some(&parse(CHAT_SCHEMA)),
        &paths,
    );
    let expected = parse(
        r##"{"openapi":"3.1.0",
        "paths":{"/v1/chat/completions":{"post":{"parameters":[],"requestBody":{"required":true,"content":{"application/json":{"schema":{
            "type":"object","required":["model","messages"],"properties":{
                "model":{"type":"string"},
                "messages":{"type":"array","items":{"$ref":"#/components/schemas/Message"},"example":[{"role":"user","content":"Hello, how are you?"}]}}}}}}}},
            "/chat/completions":{"get":{}}},
        "components":{"schemas":{
            "Message":{"type":"object","properties":{"role":{"type":"string"},"content":{"type":"string"}}},
            "ProxyChatCompletionRequest":{"type":"object","title":"ProxyChatCompletionRequest","required":["model","messages"],
                "properties":{"model":{"type":"string"},"messages":{"type":"array","items":{"$ref":"#/components/schemas/Message"}}}}}}}"##,
    );
    assert_eq!(document, expected);
}

#[test]
fn components_and_paths_compose_step_by_step() {
    let mut stepwise = parse(r#"{"paths":{"/embeddings":{"post":{}}}}"#);
    add_schema_to_components(&mut stepwise, "Chat", &parse(CHAT_SCHEMA));
    add_request_body_to_paths(&mut stepwise, &["/embeddings"], "#/components/schemas/Chat");
    let mut combined = parse(r#"{"paths":{"/embeddings":{"post":{}}}}"#);
    add_request_schema(
        &mut combined,
        "Chat",
        Some(&parse(CHAT_SCHEMA)),
        &["/embeddings"],
    );
    assert_eq!(stepwise, combined);
}

#[test]
fn inline_body_is_self_contained() {
    let body =
        inline_request_body(Some(&parse(CHAT_SCHEMA)), &parse(r#"{"model":"gpt-4o"}"#)).unwrap();
    let text = serde_json::to_string(&body).unwrap();
    assert!(!text.contains("$defs"));
    assert!(
        text.contains(
            r#""messages":{"type":"array","items":{"type":"object","properties":{"role""#
        )
    );
}

#[test]
fn inline_body_reports_missing_definition() {
    let schema = parse(r##"{"properties":{"a":{"$ref":"#/$defs/Gone"}}}"##);
    let error = inline_request_body(Some(&schema), &JsonObject::new()).unwrap_err();
    assert_eq!(
        error,
        InlineError::MissingDefinition {
            name: "Gone".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "the schema references #/$defs/Gone, which it does not define"
    );
}

#[test]
fn tree_round_trips_every_json_kind() {
    let text = r#"{"null":null,"flag":false,"integer":-3,"float":1.5,"text":"t","array":[1,"a"],"object":{"z":1,"a":2}}"#;
    let tree: JsonTree = serde_json::from_str(text).unwrap();
    assert!(
        matches!(&tree, JsonTree::Object(object) if object.get("null") == Some(&JsonTree::Null))
    );
    assert_eq!(serde_json::to_string(&tree).unwrap(), text);
}
