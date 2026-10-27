#![cfg(test)]

use rstest::rstest;

use crate::inline_expansion::{InlineError, inline_request_body};
use crate::schema_transforms::tree::{JsonObject, JsonTree};

fn parse(text: &str) -> JsonObject {
    serde_json::from_str(text).unwrap()
}

fn inlined_schema(schema: &str) -> Result<JsonTree, InlineError> {
    inline_request_body(Some(&parse(schema)), &parse(r#"{"model":"m"}"#))
        .map(|body| media_schema(&body))
}

fn media_schema(body: &JsonObject) -> JsonTree {
    let example = parse(r#"{"model":"m"}"#);
    assert_eq!(body.keys().collect::<Vec<_>>(), ["required", "content"]);
    assert_eq!(body.get("required"), Some(&JsonTree::Flag(true)));
    let Some(JsonTree::Object(content)) = body.get("content") else {
        panic!("content is an object");
    };
    let Some(JsonTree::Object(media)) = content.get("application/json") else {
        panic!("the media type is an object");
    };
    assert_eq!(media.keys().collect::<Vec<_>>(), ["schema", "example"]);
    assert_eq!(media.get("example"), Some(&JsonTree::Object(example)));
    media.get("schema").unwrap().clone()
}

fn tree(text: &str) -> JsonTree {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::reference_expanded(
    r##"{"properties":{"message":{"$ref":"#/$defs/Message"}},"$defs":{"Message":{"type":"string"}}}"##,
    r#"{"properties":{"message":{"type":"string"}}}"#
)]
#[case::siblings_override_target(
    r##"{"properties":{"a":{"$ref":"#/$defs/A","description":"own","extra":[{"$ref":"#/$defs/B"}]}},"$defs":{"A":{"type":"object","description":"shared"},"B":{"type":"integer"}}}"##,
    r#"{"properties":{"a":{"type":"object","description":"own","extra":[{"type":"integer"}]}}}"#
)]
#[case::non_object_target_keeps_siblings(
    r##"{"items":{"$ref":"#/$defs/Flag","title":"t"},"$defs":{"Flag":true}}"##,
    r#"{"items":{"title":"t"}}"#
)]
#[case::root_reference(
    r##"{"$ref":"#/$defs/Root","$defs":{"Root":{"type":"object","properties":{"child":{"$ref":"#/$defs/Leaf"}}},"Leaf":{"enum":[1,2]}}}"##,
    r#"{"type":"object","properties":{"child":{"enum":[1,2]}},"$defs":{"Root":{"type":"object","properties":{"child":{"enum":[1,2]}}},"Leaf":{"enum":[1,2]}}}"#
)]
#[case::foreign_reference_kept(
    r##"{"items":{"$ref":"#/components/schemas/Other","$defs":{"X":{}}}}"##,
    r##"{"items":{"$ref":"#/components/schemas/Other"}}"##
)]
#[case::nested_defs_removed(
    r#"{"anyOf":[{"$defs":{"X":{}},"type":"null"},null,"text",3]}"#,
    r#"{"anyOf":[{"type":"null"},null,"text",3]}"#
)]
fn references_expand_in_place(#[case] schema: &str, #[case] expected: &str) {
    assert_eq!(inlined_schema(schema), Ok(tree(expected)));
}

#[rstest]
#[case::mapping_dropped(
    r#"{"discriminator":{"propertyName":"kind","mapping":{"a":"x"},"extra":1}}"#,
    r#"{"discriminator":{"propertyName":"kind"}}"#
)]
#[case::mapping_without_property_name(
    r#"{"discriminator":{"mapping":{"a":{"$defs":{}}}}}"#,
    r#"{"discriminator":{"mapping":{"a":{}}}}"#
)]
#[case::property_name_without_mapping(
    r#"{"discriminator":{"propertyName":"kind"}}"#,
    r#"{"discriminator":{"propertyName":"kind"}}"#
)]
fn discriminator_mapping_removed(#[case] schema: &str, #[case] expected: &str) {
    assert_eq!(inlined_schema(schema), Ok(tree(expected)));
}

fn nested_reference(depth: usize) -> String {
    let reference = r##"{"$ref":"#/$defs/Missing","$defs":{"Z":{}}}"##;
    format!(
        "{}{reference}{}",
        r#"{"a":"#.repeat(depth),
        "}".repeat(depth)
    )
}

#[test]
fn reference_below_depth_limit_is_expanded() {
    let result = inlined_schema(&nested_reference(31));
    assert_eq!(
        result,
        Err(InlineError::MissingDefinition {
            name: "Missing".to_owned()
        })
    );
}

#[test]
fn reference_at_depth_limit_is_kept() {
    let schema = nested_reference(32);
    assert_eq!(inlined_schema(&schema), Ok(tree(&schema)));
}

#[test]
fn array_depth_counts_toward_limit() {
    let reference = r##"{"$ref":"#/$defs/Missing"}"##;
    let wrapped = format!(r#"{{"a":{}{reference}{}}}"#, "[".repeat(31), "]".repeat(31));
    assert_eq!(inlined_schema(&wrapped), Ok(tree(&wrapped)));
}

#[rstest]
#[case::absent_definition(r##"{"$ref":"#/$defs/Missing","$defs":{"Other":{}}}"##)]
#[case::defs_not_object(r##"{"properties":{"a":{"$ref":"#/$defs/A"}},"$defs":["A"]}"##)]
#[case::missing_in_sibling(
    r##"{"$ref":"#/$defs/A","items":{"$ref":"#/$defs/Missing"},"$defs":{"A":{}}}"##
)]
fn missing_reference_raises(#[case] schema: &str) {
    let result = inlined_schema(schema);
    assert!(matches!(result, Err(InlineError::MissingDefinition { .. })));
}

#[test]
fn self_reference_stops_at_depth_limit() {
    let schema = r##"{"properties":{"head":{"$ref":"#/$defs/Node"}},"$defs":{"Node":{"properties":{"next":{"$ref":"#/$defs/Node"}}}}}"##;
    let chain = format!(
        r##"{}{{"$ref":"#/$defs/Node"}}{}"##,
        r#"{"properties":{"next":"#.repeat(10),
        "}}".repeat(10)
    );
    let expected = format!(r#"{{"properties":{{"head":{chain}}}}}"#);
    assert_eq!(inlined_schema(schema), Ok(tree(&expected)));
}

#[test]
fn failed_generation_keeps_example_only() {
    let body = inline_request_body(None, &parse(r#"{"input":"hi"}"#)).unwrap();
    assert_eq!(
        body,
        parse(r#"{"required":true,"content":{"application/json":{"example":{"input":"hi"}}}}"#)
    );
}
