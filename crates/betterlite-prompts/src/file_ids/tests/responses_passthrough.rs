use super::*;
use crate::file_ids::responses::{ResponsesInput, update_responses_file_ids};
use rstest::rstest;

fn responses_input(text: &str) -> ResponsesInput {
    serde_json::from_str(text).expect("test responses input decodes")
}

#[rstest]
#[case::string_input(r#""just text""#, r#""just text""#)]
#[case::string_content(
    r#"[{"role":"user","content":"hello"}]"#,
    r#"[{"role":"user","content":"hello"}]"#
)]
#[case::item_without_content(
    r#"[{"type":"function_call","call_id":"call-1"}]"#,
    r#"[{"type":"function_call","call_id":"call-1"}]"#
)]
#[case::non_input_file_parts(
    r#"[{"role":"user","content":[{"type":"input_text","text":"hi"},{"type":"input_image","image_url":"https://example.com/x.png"}]}]"#,
    r#"[{"role":"user","content":[{"type":"input_text","text":"hi"},{"type":"input_image","image_url":"https://example.com/x.png"}]}]"#
)]
#[case::input_file_without_a_file_id(
    r#"[{"role":"user","content":[{"type":"input_file"}]}]"#,
    r#"[{"role":"user","content":[{"type":"input_file"}]}]"#
)]
#[case::empty_content_list(
    r#"[{"role":"user","content":[]}]"#,
    r#"[{"role":"user","content":[]}]"#
)]
#[case::empty_input("[]", "[]")]
fn model_file_ids_behavior_tests_behavior(#[case] input: &str, #[case] expected: &str) {
    let updated = update_responses_file_ids(
        &responses_input(input),
        Some(DEPLOYMENT),
        &FileIdMapping::default(),
    )
    .expect("the input stays within the limits");
    assert_eq!(serialized(&updated), json(expected));
}
