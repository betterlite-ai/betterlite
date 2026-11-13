#![cfg(test)]

use rstest::rstest;

use super::audio::estimate_audio_content_tokens;
use super::support::value;

#[test]
fn no_messages_estimate_no_audio() {
    assert_eq!(estimate_audio_content_tokens(None), 0);
}

#[rstest]
#[case::not_a_list(r#""messages""#, 0)]
#[case::object(r#"{"content":[]}"#, 0)]
#[case::no_audio(r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#, 0)]
#[case::string_content(r#"[{"role":"user","content":"hi"}]"#, 0)]
#[case::non_object_block(r#"[{"role":"user","content":[5]}]"#, 0)]
#[case::missing_content(r#"[{"role":"user"}]"#, 0)]
#[case::one_reference_block(
    r#"[{"role":"user","content":[{"type":"input_audio","input_audio":{}}]}]"#,
    300
)]
#[case::two_messages(
    r#"[{"role":"user","content":[{"type":"input_audio","input_audio":{}}]},{"role":"user","content":[{"type":"input_audio","input_audio":{}}]}]"#,
    600
)]
fn audio_blocks_sum_across_messages(#[case] messages: &str, #[case] expected: i64) {
    assert_eq!(
        estimate_audio_content_tokens(Some(&value(messages))),
        expected
    );
}
