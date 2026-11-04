#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::disallowed_methods,
        clippy::default_numeric_fallback,
        reason = "json! fixtures make expected heterogeneous content explicit"
    )
)]

use super::annotations::{AnnotationValue, annotation_fallback};
use super::content::{content_passthrough, text_by_role};
use super::image_file::image_and_file_fields;
use super::tool_choice::flat_tool_choice;
use rstest::rstest;
use serde_json::{Value, json};

#[test]
fn text_by_role_defaults_null_and_unknown_content() {
    assert_eq!(
        text_by_role(&Value::Null, "user").unwrap(),
        json!([{"type":"input_text","text":""}])
    );
    assert_eq!(
        text_by_role(&json!(7), "other").unwrap(),
        json!([{"type":"output_text","text":"7"}])
    );
    assert_eq!(
        text_by_role(&json!([{"type":"text"}, 7, "hi"]), "assistant").unwrap(),
        json!([{"type":"output_text","text":""},{"type":"output_text","text":"hi"}])
    );
}

#[rstest]
#[case::user("user", "input_text")]
#[case::system("system", "input_text")]
#[case::developer("developer", "input_text")]
#[case::tool("tool", "input_text")]
#[case::assistant("assistant", "output_text")]
#[case::unknown("new_role", "output_text")]
fn role_selects_text_block_kind(#[case] role: &str, #[case] kind: &str) {
    assert_eq!(
        text_by_role(&json!("hi"), role).unwrap(),
        json!([{"type":kind,"text":"hi"}])
    );
}

#[test]
fn text_by_role_projects_standard_assistant_content_parts() {
    assert_eq!(
        text_by_role(
            &json!([
                {"type":"image_url","image_url":{"url":"u"}},
                {"type":"input_text","text":"x"},
                {"type":"file","file":{"file_id":"f"}},
                {"type":"tool_reference"},
                {"type":"other"}
            ]),
            "assistant"
        )
        .unwrap(),
        json!([
            {"type":"input_image","image_url":"u","detail":"auto"},
            {"type":"input_text","text":"x"},
            {"type":"input_file","file_id":"f"},
            {"type":"output_text","text":"{'type': 'other'}"}
        ])
    );
}

#[test]
fn text_by_role_propagates_invalid_image_url() {
    assert_eq!(
        text_by_role(&json!([{"type":"image_url"}]), "assistant")
            .unwrap_err()
            .to_string(),
        "Invalid image URL: None"
    );
}

#[test]
fn content_passthrough_keeps_spread_order_and_skips_tool_reference() {
    let projection = content_passthrough(
        &[
            json!({"type":"image","url":"u"}),
            json!({"type":"tool_reference"}),
            json!({"other":1}),
        ],
        "user",
    )
    .expect("content is projectable");
    assert_eq!(
        projection.value,
        json!([{"type":"image","url":"u"},{"other":1}])
    );
    assert!(
        projection
            .diagnostics
            .iter()
            .any(|entry| entry.contains("tool_reference"))
    );
}

#[test]
fn content_passthrough_emits_fixture_final_diagnostic() {
    let projection = content_passthrough(
        &[
            json!({"type":"image","url":"u"}),
            json!({"type":"tool_reference"}),
            json!({"type":"input_text","text":"hi"}),
            json!({"type":"mystery","text":"hey"}),
        ],
        "user",
    )
    .expect("content is projectable");
    assert_eq!(
        projection.diagnostics.last().expect("final log"),
        "Chat provider: Final converted content: [{'type': 'image', 'url': 'u'}, {'text': 'hi', 'type': 'input_text'}, {'type': 'input_text', 'text': 'hey'}]"
    );
}

#[test]
fn content_passthrough_projects_standard_chat_image_url_blocks() {
    let projection = content_passthrough(
        &[json!({
            "type":"image_url",
            "image_url":{"url":"https://example.test/a","detail":"high"}
        })],
        "user",
    )
    .expect("image URL is valid");
    assert_eq!(
        projection.value,
        json!([{
            "image_url":"https://example.test/a",
            "detail":"high",
            "type":"input_image"
        }])
    );
}

#[test]
fn image_and_file_projection_keeps_detail_and_fields() {
    let image =
        serde_json::from_value(json!({"image_url":{"url":"u","detail":"high"}})).expect("map");
    assert_eq!(
        image_and_file_fields(&image).expect("image"),
        json!({"type":"input_image","image_url":"u","detail":"high"})
    );
}

#[test]
fn image_and_file_projection_preserves_present_file_fields_only() {
    let file = serde_json::from_value(json!({
        "type":"file",
        "file":{"file_id":null,"file_data":"","filename":"f","file_url":"drop","vendor":1}
    }))
    .expect("map");
    assert_eq!(
        image_and_file_fields(&file).expect("file"),
        json!({
            "type":"input_file","file_id":null,"file_data":"","filename":"f"
        })
    );
}

#[rstest]
#[case::missing_file(json!({"type":"file"}))]
#[case::non_dict_file(json!({"type":"file","file":"x"}))]
fn image_and_file_projection_defaults_missing_or_non_dict_file(#[case] input: Value) {
    let item = serde_json::from_value(input).expect("map");
    assert_eq!(
        image_and_file_fields(&item).unwrap(),
        json!({"type":"input_file"})
    );
}

#[test]
fn image_and_file_projection_defaults_empty_detail_and_reports_missing_url() {
    let missing = serde_json::from_value(json!({})).expect("map");
    assert_eq!(
        image_and_file_fields(&missing).unwrap_err().to_string(),
        "Invalid image URL: None"
    );
    let image = serde_json::from_value(json!({"image_url":{"url":"","detail":""}})).expect("map");
    assert_eq!(
        image_and_file_fields(&image).expect("empty URL is accepted"),
        json!({"type":"input_image","image_url":"","detail":"auto"})
    );
    let null_url = serde_json::from_value(json!({"image_url":{"url":null}})).expect("map");
    assert_eq!(
        image_and_file_fields(&null_url)
            .expect_err("null URL is invalid")
            .to_string(),
        "Invalid image URL: {'url': None}"
    );
}

#[test]
fn flat_choice_prefers_top_level_name_and_preserves_invalid() {
    assert_eq!(
        flat_tool_choice(
            &json!({"type":"function","name":"top","function":{"name":"nested"},"x":1})
        ),
        json!({"type":"function","name":"top"})
    );
    assert_eq!(
        flat_tool_choice(&json!({"type":"function","name":""})),
        json!({"type":"function","name":""})
    );
}

#[rstest]
#[case::unknown_type(json!({"type":"allowed_tools","name":"x","extra":1}))]
#[case::missing_type(json!({"name":"x"}))]
#[case::non_string_type(json!({"type":7,"name":"x"}))]
#[case::non_object(json!("auto"))]
fn flat_choice_passes_through_unrecognized_choice(#[case] choice: Value) {
    assert_eq!(flat_tool_choice(&choice), choice);
}

#[rstest]
#[case::auto("auto")]
#[case::none("none")]
#[case::required("required")]
fn flat_choice_short_forms(#[case] choice: &str) {
    assert_eq!(
        flat_tool_choice(&json!({"type":choice,"extra":1})),
        json!(choice)
    );
}

#[test]
fn flat_custom_choice_uses_nested_name_and_keeps_invalid_name() {
    assert_eq!(
        flat_tool_choice(&json!({"type":"custom","custom":{"name":"shell"}})),
        json!({"type":"custom","name":"shell"})
    );
    assert_eq!(
        flat_tool_choice(&json!({"type":"custom","name":7,"custom":{"name":""}})),
        json!({"type":"custom","name":7,"custom":{"name":""}})
    );
}

#[test]
fn flat_choice_flattens_function_nested_name_and_custom_top_level_name() {
    assert_eq!(
        flat_tool_choice(&json!({"type":"function","function":{"name":"n"}})),
        json!({"type":"function","name":"n"})
    );
    assert_eq!(
        flat_tool_choice(&json!({"type":"custom","name":"t","custom":{"name":"n"}})),
        json!({"type":"custom","name":"t"})
    );
}

#[test]
fn annotation_fallback_keeps_valid_order_and_logs_skips() {
    let annotations = [
        AnnotationValue::Dict(json!({"type":"url_citation"})),
        AnnotationValue::Unsupported("int".to_owned()),
        AnnotationValue::Raises("ValueError('broken')".to_owned()),
    ];
    let result = annotation_fallback(Some(&annotations));
    assert_eq!(result.value, json!([{"type":"url_citation"}]));
    assert_eq!(result.diagnostics.len(), 2);
}

#[test]
fn annotation_fallback_returns_none_for_empty_inputs() {
    assert_eq!(annotation_fallback(None).value, Value::Null);
    assert_eq!(annotation_fallback(Some(&[])).value, Value::Null);
    let skipped = [AnnotationValue::Unsupported("int".to_owned())];
    assert_eq!(annotation_fallback(Some(&skipped)).value, Value::Null);
}
