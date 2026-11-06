use super::*;
use crate::file_ids::messages::get_file_ids_from_messages;
use rstest::rstest;

#[rstest]
#[case::collects_user_file_ids_in_order(
    r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"file-a"}},{"type":"text","text":"hi"},{"type":"file","file":{"file_id":"file-b"}}]}]"#,
    &["file-a", "file-b"]
)]
#[case::skips_non_user_messages(
    r#"[{"role":"system","content":[{"type":"file","file":{"file_id":"file-system"}}]},{"role":"assistant","content":[{"type":"file","file":{"file_id":"file-assistant"}}]}]"#,
    &[]
)]
#[case::skips_file_parts_without_a_file_object(
    r#"[{"role":"user","content":[{"type":"file"},{"type":"file","file":"not-a-dict"},{"type":"file","file":{}}]}]"#,
    &[]
)]
#[case::skips_an_empty_file_id(
    r#"[{"role":"user","content":[{"type":"file","file":{"file_id":""}}]}]"#,
    &[]
)]
#[case::skips_string_content(r#"[{"role":"user","content":"plain string"}]"#, &[])]
#[case::keeps_duplicates(
    r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"file-a"}},{"type":"file","file":{"file_id":"file-a"}}]}]"#,
    &["file-a", "file-a"]
)]
#[case::collects_across_messages(
    r#"[{"role":"user","content":[{"type":"file","file":{"file_id":"file-a"}}]},{"role":"user","content":[{"type":"file","file":{"file_id":"file-b"}}]}]"#,
    &["file-a", "file-b"]
)]
fn model_file_ids_behavior_tests_behavior(#[case] input: &str, #[case] expected: &[&str]) {
    let expected_ids: Vec<String> = expected.iter().map(|id| (*id).to_owned()).collect();
    let file_ids = get_file_ids_from_messages(&messages(input))
        .expect("the message list stays within the limits");
    assert_eq!(file_ids, expected_ids);
}
