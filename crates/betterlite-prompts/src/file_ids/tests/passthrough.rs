use super::*;
use crate::file_ids::messages::update_messages_with_file_ids;
use rstest::rstest;

const MAPPING: &str = r#"{"file-part":{"deployment-1":"provider-file-9"}}"#;

#[rstest]
#[case::string_content(
    r#"[{"role":"user","content":"plain string"}]"#,
    r#"[{"role":"user","content":"plain string"}]"#
)]
#[case::non_file_parts(
    r#"[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"https://example.com/x.png"}}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":{"url":"https://example.com/x.png"}}]}]"#
)]
#[case::file_part_without_the_file_object(
    r#"[{"role":"user","content":[{"type":"file"}]}]"#,
    r#"[{"role":"user","content":[{"type":"file"}]}]"#
)]
#[case::file_part_with_a_non_object_file(
    r#"[{"role":"user","content":[{"type":"file","file":"not-a-dict"}]}]"#,
    r#"[{"role":"user","content":[{"type":"file","file":"not-a-dict"}]}]"#
)]
#[case::assistant_and_system_file_parts(
    r#"[{"role":"system","content":[{"type":"file","file":{"file_id":"file-part"}}]},{"role":"assistant","content":[{"type":"file","file":{"file_id":"file-part"}}]}]"#,
    r#"[{"role":"system","content":[{"type":"file","file":{"file_id":"file-part"}}]},{"role":"assistant","content":[{"type":"file","file":{"file_id":"file-part"}}]}]"#
)]
#[case::message_without_content(r#"[{"role":"user"}]"#, r#"[{"role":"user"}]"#)]
#[case::empty_content_list(
    r#"[{"role":"user","content":[]}]"#,
    r#"[{"role":"user","content":[]}]"#
)]
fn model_file_ids_behavior_tests_behavior(#[case] input: &str, #[case] expected: &str) {
    let mut updated = messages(input);
    update_messages_with_file_ids(&mut updated, Some(DEPLOYMENT), &mapping(MAPPING))
        .expect("the message list stays within the limits");
    assert_eq!(serialized(&updated), json(expected));
}
