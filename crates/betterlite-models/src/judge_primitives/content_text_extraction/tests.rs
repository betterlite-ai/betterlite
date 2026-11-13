#![cfg(test)]

use rstest::rstest;

use super::extract_text_from_content;
use crate::decisions::Json;

fn content(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::plain_text(r#""hello""#, "hello")]
#[case::text_keeps_its_spacing(r#""  spaced  ""#, "  spaced  ")]
#[case::empty_text(r#""""#, "")]
#[case::one_text_part(r#"[{"type":"text","text":"hi"}]"#, "hi")]
#[case::several_text_parts(r#"[{"type":"text","text":"a"},{"type":"text","text":"b"}]"#, "a b")]
#[case::skips_a_non_text_part(r#"[{"type":"image_url"},{"type":"text","text":"a"}]"#, "a")]
#[case::skips_a_string_part(r#"["a",{"type":"text","text":"b"}]"#, "b")]
#[case::skips_a_part_with_no_text(r#"[{"type":"text"},{"type":"text","text":"b"}]"#, "b")]
#[case::skips_a_null_type(r#"[{"type":null,"text":"a"},{"type":"text","text":"b"}]"#, "b")]
#[case::skips_non_string_text(r#"[{"type":"text","text":3},{"type":"text","text":"b"}]"#, "b")]
#[case::skips_a_nested_list_text(r#"[{"type":"text","text":["a"]}]"#, "")]
#[case::keeps_an_empty_text_part(r#"[{"type":"text","text":""}]"#, "")]
#[case::empty_list("[]", "")]
#[case::object_content(r#"{"type":"text","text":"a"}"#, "")]
#[case::null_content("null", "")]
#[case::number_content("3", "")]
#[case::boolean_content("true", "")]
fn extracts_text_from_content_types(#[case] source: &str, #[case] expected: &str) {
    assert_eq!(extract_text_from_content(&content(source)), expected);
}
