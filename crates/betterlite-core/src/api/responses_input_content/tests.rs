#![cfg(test)]

use super::{
    ContentError, chat_content_type, content_blocks, input_file_block, input_image_block,
    instructions_to_system_message, resolve_file_id,
};
use rstest::rstest;
use serde_json::{Map, Value, json};

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        other @ (Value::Null
        | Value::Bool(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Array(_)) => {
            panic!("expected an object, got {other:?}")
        }
    }
}

#[rstest]
#[case::explicit_file_id(json!({"file_id": "file_a", "file_url": "https://a.test/f"}), Some(json!("file_a")))]
#[case::falls_back_to_url(json!({"file_id": "", "file_url": "https://a.test/f"}), Some(json!("https://a.test/f")))]
#[case::absent_file_id(json!({"file_url": "https://a.test/f"}), Some(json!("https://a.test/f")))]
#[case::neither_present(json!({}), None)]
fn resolve_file_id_cases(#[case] item: Value, #[case] expected: Option<Value>) {
    assert_eq!(resolve_file_id(&object(item)), expected);
}

#[rstest]
fn input_file_block_full() {
    let item = object(json!({
        "file_id": "",
        "file_url": "https://a.test/f",
        "file_data": "data",
        "filename": "f",
        "cache_control": null,
    }));
    let expected = json!({
        "type": "file",
        "file": {"file_id": "https://a.test/f", "file_data": "data", "filename": "f"},
        "cache_control": null,
    });
    assert_eq!(input_file_block(&item), expected);
}

#[rstest]
fn input_file_block_omits_falsy_fields() {
    let item = object(json!({"file_id": "", "file_url": ""}));
    let expected = json!({"type": "file", "file": {}});
    assert_eq!(input_file_block(&item), expected);
}

#[rstest]
fn input_file_block_keeps_cache_control_only_when_present() {
    let item = object(json!({"file_id": "file_a"}));
    let expected = json!({"type": "file", "file": {"file_id": "file_a"}});
    assert_eq!(input_file_block(&item), expected);
}

#[rstest]
fn input_image_block_defaults() {
    let item = object(json!({"image_url": "", "detail": ""}));
    let expected = json!({"type": "image_url", "image_url": {"url": "", "detail": "auto"}});
    assert_eq!(input_image_block(&item), expected);
}

#[rstest]
fn input_image_block_supplied_values_survive() {
    let item = object(json!({"image_url": "https://a.test/i.png", "detail": "high"}));
    let expected = json!({
        "type": "image_url",
        "image_url": {"url": "https://a.test/i.png", "detail": "high"},
    });
    assert_eq!(input_image_block(&item), expected);
}

#[rstest]
#[case::null_becomes_empty_string(json!(null), Ok(json!("")))]
#[case::string_unchanged(json!("hello"), Ok(json!("hello")))]
#[case::top_level_int(json!(1_i32), Err(ContentError::InvalidContentType { type_name: "<class 'int'>" }))]
#[case::top_level_float(json!(1.5_f64), Err(ContentError::InvalidContentType { type_name: "<class 'float'>" }))]
#[case::top_level_bool(json!(true), Err(ContentError::InvalidContentType { type_name: "<class 'bool'>" }))]
#[case::top_level_dict(json!({"a": 1_i32}), Err(ContentError::InvalidContentType { type_name: "<class 'dict'>" }))]
fn content_blocks_scalar_cases(
    #[case] content: Value,
    #[case] expected: Result<Value, ContentError>,
) {
    assert_eq!(content_blocks(&content), expected);
}

#[rstest]
fn content_blocks_preserves_list_order_and_drops_non_string_non_dict() {
    let content = json!(["a", 5_i32, "b", true, {"type": "input_file", "file_id": "file_a"}]);
    let result = content_blocks(&content).expect("valid list content");
    let expected = json!([
        "a",
        "b",
        {"type": "file", "file": {"file_id": "file_a"}},
    ]);
    assert_eq!(result, expected);
}

#[rstest]
fn content_blocks_encrypted_content_stringifies_non_null_value() {
    let content = json!([{"type": "encrypted_content", "encrypted_content": 7_i32}]);
    let result = content_blocks(&content).expect("valid list content");
    assert_eq!(result, json!([{"type": "text", "text": "7"}]));
}

#[rstest]
fn content_blocks_encrypted_content_drops_null_value() {
    let content = json!([{"type": "encrypted_content", "encrypted_content": null}]);
    let result = content_blocks(&content).expect("valid list content");
    assert_eq!(result, json!([]));
}

#[rstest]
fn content_blocks_drops_text_blocks_with_null_text() {
    let content = json!([{"type": "text", "text": null}]);
    let result = content_blocks(&content).expect("valid list content");
    assert_eq!(result, json!([]));
}

#[rstest]
fn content_blocks_keeps_empty_text_and_cache_control() {
    let content = json!([{"type": "text", "text": "", "cache_control": {"type": "ephemeral"}}]);
    let result = content_blocks(&content).expect("valid list content");
    assert_eq!(
        result,
        json!([{"type": "text", "text": "", "cache_control": {"type": "ephemeral"}}])
    );
}

#[rstest]
fn content_blocks_copies_cache_control_onto_image_even_when_null() {
    let content = json!([{"type": "input_image", "image_url": "https://a.test/i.png", "cache_control": null}]);
    let result = content_blocks(&content).expect("valid list content");
    assert_eq!(
        result,
        json!([{
            "type": "image_url",
            "image_url": {"url": "https://a.test/i.png", "detail": "auto"},
            "cache_control": null,
        }])
    );
}

#[rstest]
#[case::input_text("input_text", "text")]
#[case::output_text("output_text", "text")]
#[case::tool_result("tool_result", "text")]
#[case::unknown("something_else", "text")]
#[case::input_audio("input_audio", "input_audio")]
#[case::input_image_url("input_image_url", "image_url")]
#[case::input_document("input_document", "document")]
#[case::input_guarded_text("input_guarded_text", "guarded_text")]
#[case::input_video_url("input_video_url", "video_url")]
#[case::input_file("input_file", "file")]
#[case::already_text("text", "text")]
#[case::already_image_url("image_url", "image_url")]
#[case::already_input_audio("input_audio", "input_audio")]
#[case::already_audio_url("audio_url", "audio_url")]
#[case::already_document("document", "document")]
#[case::already_guarded_text("guarded_text", "guarded_text")]
#[case::already_video_url("video_url", "video_url")]
#[case::already_file("file", "file")]
#[case::already_thinking("thinking", "thinking")]
#[case::already_redacted_thinking("redacted_thinking", "redacted_thinking")]
fn chat_content_type_cases(#[case] content_type: &str, #[case] expected: &str) {
    assert_eq!(chat_content_type(content_type), expected);
}

#[rstest]
#[case::null(None, "")]
#[case::empty(Some(""), "")]
#[case::present(Some("follow"), "follow")]
fn instructions_to_system_message_cases(#[case] instructions: Option<&str>, #[case] content: &str) {
    assert_eq!(
        instructions_to_system_message(instructions),
        json!({"role": "system", "content": content})
    );
}
