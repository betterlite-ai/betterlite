#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::inline_expansion::inline_request_body;
use crate::schema_transforms::tree::{JsonObject, JsonTree};
use crate::schema_transforms::{
    add_request_body_to_paths, add_request_schema, add_schema_to_components,
};

fn parse(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::identical(
    r#"{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}"#
)]
#[case::same_names_other_types(
    r#"{"type":"object","properties":{"value":{"type":"integer"}},"required":["value"]}"#
)]
#[case::other_keywords_ignored(
    r#"{"title":"Other","properties":{"value":{}},"required":["value",7]}"#
)]
fn same_shape_reuses_component_on_collision(#[case] incoming: &str) {
    let existing =
        r#"{"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}"#;
    let mut document = parse(&format!(
        r#"{{"components":{{"schemas":{{"Block":{existing}}}}}}}"#
    ));
    let schema = parse(&format!(
        r##"{{"properties":{{"block":{{"$ref":"#/$defs/Block"}}}},"$defs":{{"Block":{incoming}}}}}"##
    ));
    add_schema_to_components(&mut document, "Request", &schema);
    let expected = parse(&format!(
        r##"{{"components":{{"schemas":{{"Block":{existing},"Request":{{"properties":{{"block":{{"$ref":"#/components/schemas/Block"}}}}}}}}}}}}"##
    ));
    assert_eq!(document, expected);
}

#[rstest]
#[case::other_property_names(r#"{"properties":{"other":{"type":"string"}},"required":["value"]}"#)]
#[case::other_required(r#"{"properties":{"value":{"type":"string"}},"required":[]}"#)]
#[case::missing_required(r#"{"properties":{"value":{"type":"string"}}}"#)]
#[case::no_properties(r#"{"type":"string"}"#)]
#[case::empty_properties(r#"{"properties":{},"required":["value"]}"#)]
fn different_shape_renames_on_collision(#[case] incoming: &str) {
    let existing = r#"{"properties":{"value":{"type":"string"}},"required":["value"]}"#;
    let mut document = parse(&format!(
        r#"{{"components":{{"schemas":{{"Block":{existing}}}}}}}"#
    ));
    let schema = parse(&format!(
        r##"{{"properties":{{"block":{{"$ref":"#/$defs/Block"}}}},"$defs":{{"Block":{incoming}}}}}"##
    ));
    add_schema_to_components(&mut document, "Request", &schema);
    let expected = parse(&format!(
        r##"{{"components":{{"schemas":{{"Block":{existing},"Request_Block":{incoming},"Request":{{"properties":{{"block":{{"$ref":"#/components/schemas/Request_Block"}}}}}}}}}}}}"##
    ));
    assert_eq!(document, expected);
}

#[test]
fn empty_existing_properties_rename_on_collision() {
    let mut document = parse(r#"{"components":{"schemas":{"Block":{"properties":{}}}}}"#);
    let schema = parse(r#"{"$defs":{"Block":{"properties":{"value":{}}}}}"#);
    add_schema_to_components(&mut document, "Request", &schema);
    let expected = parse(
        r#"{"components":{"schemas":{"Block":{"properties":{}},"Request_Block":{"properties":{"value":{}}},"Request":{}}}}"#,
    );
    assert_eq!(document, expected);
}

#[test]
fn renamed_reference_reaches_fixed_point_collision() {
    let mut document = parse(
        r##"{"components":{"schemas":{
            "Outer":{"$ref":"#/components/schemas/Inner"},
            "Inner":{"properties":{"kept":{}}}
        }}}"##,
    );
    let schema = parse(
        r##"{"$ref":"#/$defs/Outer","$defs":{
            "Outer":{"$ref":"#/$defs/Inner"},
            "Inner":{"properties":{"changed":{}}},
            "Fresh":{"type":"integer"}
        }}"##,
    );
    add_schema_to_components(&mut document, "Chat", &schema);
    let expected = parse(
        r##"{"components":{"schemas":{
            "Outer":{"$ref":"#/components/schemas/Inner"},
            "Inner":{"properties":{"kept":{}}},
            "Chat_Outer":{"$ref":"#/components/schemas/Chat_Inner"},
            "Chat_Inner":{"properties":{"changed":{}}},
            "Fresh":{"type":"integer"},
            "Chat":{"$ref":"#/components/schemas/Chat_Outer"}
        }}}"##,
    );
    assert_eq!(document, expected);
}

#[test]
fn equal_definition_without_properties_reuses_on_collision() {
    let mut document = parse(r#"{"components":{"schemas":{"Mode":{"enum":["a","b"]}}}}"#);
    let schema = parse(r#"{"$defs":{"Mode":{"enum":["a","b"]}}}"#);
    add_schema_to_components(&mut document, "Request", &schema);
    let expected = parse(r#"{"components":{"schemas":{"Mode":{"enum":["a","b"]},"Request":{}}}}"#);
    assert_eq!(document, expected);
}

#[rstest]
#[case::nested_and_arrays(
    r##"{"anyOf":[{"$ref":"#/$defs/A"},{"items":{"$ref":"#/$defs/B","$defs":{"X":{}}}}],"$defs":{"A":{"type":"string"},"B":{"$ref":"#/$defs/A"}}}"##,
    r##"{"A":{"type":"string"},"B":{"$ref":"#/components/schemas/A"},"Request":{"anyOf":[{"$ref":"#/components/schemas/A"},{"items":{"$ref":"#/components/schemas/B"}}]}}"##
)]
#[case::foreign_refs_kept(
    r##"{"properties":{"a":{"$ref":"#/components/schemas/Other"},"b":{"$ref":5},"c":{"title":"#/$defs/A"}}}"##,
    r##"{"Request":{"properties":{"a":{"$ref":"#/components/schemas/Other"},"b":{"$ref":5},"c":{"title":"#/$defs/A"}}}}"##
)]
#[case::orphan_reference_rewritten(
    r##"{"$ref":"#/$defs/Missing"}"##,
    r##"{"Request":{"$ref":"#/components/schemas/Missing"}}"##
)]
#[case::every_prefix_stripped(
    r##"{"$ref":"#/$defs/A#/$defs/B"}"##,
    r##"{"Request":{"$ref":"#/components/schemas/AB"}}"##
)]
#[case::non_object_defs_ignored(
    r#"{"$defs":[1],"type":"object"}"#,
    r#"{"Request":{"type":"object"}}"#
)]
fn defs_reference_rewrite(#[case] schema: &str, #[case] schemas: &str) {
    let mut document = JsonObject::new();
    add_schema_to_components(&mut document, "Request", &parse(schema));
    let expected = parse(&format!(r#"{{"components":{{"schemas":{schemas}}}}}"#));
    assert_eq!(document, expected);
}

#[rstest]
#[case::components_not_object(r#"{"components":[]}"#)]
#[case::schemas_not_object(r#"{"components":{"schemas":"none"}}"#)]
fn non_object_components_skip_rewrite(#[case] document: &str) {
    let mut openapi = parse(document);
    add_schema_to_components(&mut openapi, "Request", &parse(r#"{"$defs":{"A":{}}}"#));
    assert_eq!(openapi, parse(document));
}

#[test]
fn request_body_expands_fields_and_keeps_path_parameters() {
    let mut document = parse(
        r##"{"components":{"schemas":{"Request":{
            "required":["model"],
            "properties":{
                "model":{"type":"string","$defs":{"Z":{}}},
                "messages":{"type":"array","example":"old","items":{"$ref":"#/$defs/Message"}},
                "flag":true
            }}}},
        "paths":{
            "/chat":{"post":{"parameters":[{"in":"path","name":"model"},{"in":"query","name":"q"},"bare"],"requestBody":{}}},
            "/flat":{"post":{"parameters":{"in":"path"}}},
            "/get":{"get":{}},
            "/odd":{"post":[]},
            "/plain":"text"
        }}"##,
    );
    add_request_body_to_paths(
        &mut document,
        &["/chat", "/flat", "/get", "/odd", "/plain", "/absent"],
        "#/components/schemas/Request",
    );
    let body = r##"{"required":true,"content":{"application/json":{"schema":{"type":"object","required":["model"],"properties":{
        "model":{"type":"string"},
        "messages":{"type":"array","example":[{"role":"user","content":"Hello, how are you?"}],"items":{"$ref":"#/components/schemas/Message"}},
        "flag":{}
    }}}}}"##;
    let paths = parse(&format!(
        r#"{{"/chat":{{"post":{{"parameters":[{{"in":"path","name":"model"}}],"requestBody":{body}}}}},
        "/flat":{{"post":{{"parameters":[],"requestBody":{body}}}}},
        "/get":{{"get":{{}}}},"/odd":{{"post":[]}},"/plain":"text"}}"#
    ));
    assert_eq!(document.get("paths"), Some(&JsonTree::Object(paths)));
}

#[rstest]
#[case::unknown_schema(
    "#/components/schemas/Missing",
    r#"{"components":{"schemas":{}},"paths":{"/chat":{"post":{}}}}"#
)]
#[case::no_components("Request", r#"{"paths":{"/chat":{"post":{}}}}"#)]
fn missing_schema_gives_empty_body_fallback(#[case] schema_ref: &str, #[case] document: &str) {
    let mut openapi = parse(document);
    add_request_body_to_paths(&mut openapi, &["/chat"], schema_ref);
    let body = r#"{"required":true,"content":{"application/json":{"schema":{"type":"object","required":[],"properties":{}}}}}"#;
    let post = format!(r#"{{"post":{{"requestBody":{body}}}}}"#);
    assert_eq!(
        openapi
            .get("paths")
            .map(|paths| serde_json::to_string(paths).unwrap()),
        Some(format!(r#"{{"/chat":{post}}}"#))
    );
}

#[test]
fn document_without_paths_is_left_alone() {
    let mut document = parse(r#"{"components":{"schemas":{"Request":{"properties":{"a":{}}}}}}"#);
    add_request_body_to_paths(&mut document, &["/chat"], "#/components/schemas/Request");
    assert_eq!(
        document,
        parse(r#"{"components":{"schemas":{"Request":{"properties":{"a":{}}}}}}"#)
    );
}

#[test]
fn generated_schema_decorates_components_and_paths() {
    let mut document = parse(r#"{"paths":{"/v1/embeddings":{"post":{}}}}"#);
    let schema = parse(
        r##"{"properties":{"input":{"$ref":"#/$defs/Input"}},"required":["input"],"$defs":{"Input":{"type":"string"}}}"##,
    );
    add_request_schema(
        &mut document,
        "EmbeddingRequest",
        Some(&schema),
        &["/v1/embeddings"],
    );
    let expected = parse(
        r##"{"paths":{"/v1/embeddings":{"post":{"requestBody":{"required":true,"content":{"application/json":{"schema":{
            "type":"object","required":["input"],"properties":{"input":{"$ref":"#/components/schemas/Input"}}}}}}}}},
        "components":{"schemas":{"Input":{"type":"string"},"EmbeddingRequest":{"properties":{"input":{"$ref":"#/components/schemas/Input"}},"required":["input"]}}}}"##,
    );
    assert_eq!(document, expected);
}

#[test]
fn failed_generation_leaves_document_fallback() {
    let text = r#"{"paths":{"/v1/embeddings":{"post":{"parameters":[{"in":"query"}]}}}}"#;
    let mut document = parse(text);
    add_request_schema(&mut document, "EmbeddingRequest", None, &["/v1/embeddings"]);
    assert_eq!(document, parse(text));
}

#[test]
fn failed_generation_gives_example_body_fallback() {
    let body = inline_request_body(None, &parse(r#"{"model":"gpt"}"#)).unwrap();
    let expected =
        parse(r#"{"required":true,"content":{"application/json":{"example":{"model":"gpt"}}}}"#);
    assert_eq!(body, expected);
}

#[test]
fn replays_the_openapi_schema_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openapi_schema");
    for entry in fs::read_dir(corpus).unwrap() {
        let text = fs::read_to_string(entry.unwrap().path()).unwrap();
        let Ok(schema) = serde_json::from_str::<JsonObject>(&text) else {
            continue;
        };
        let mut document = parse(r#"{"paths":{"/chat":{"post":{"parameters":[{"in":"path"}]}}}}"#);
        add_request_schema(&mut document, "Corpus", Some(&schema), &["/chat"]);
        drop(inline_request_body(Some(&schema), &JsonObject::new()));
        assert!(document.contains_key("components"));
    }
}
