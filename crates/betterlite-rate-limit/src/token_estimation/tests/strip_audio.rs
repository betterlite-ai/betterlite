#![cfg(test)]

use rstest::rstest;

use super::audio::strip_audio_content_blocks;
use super::support::value;

#[rstest]
#[case::not_a_list(r#""messages""#, r#""messages""#)]
#[case::null("null", "null")]
#[case::without_audio(
    r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#
)]
#[case::with_audio(
    r#"[{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"AA"}},{"type":"text","text":"hi"}]}]"#,
    r#"[{"role":"user","content":[{"type":"text","text":"hi"}]}]"#
)]
#[case::other_keys_kept(
    r#"[{"role":"user","name":"n","content":[{"type":"input_audio"}]}]"#,
    r#"[{"role":"user","name":"n","content":[]}]"#
)]
#[case::string_content_kept(
    r#"[{"role":"user","content":"hi"}]"#,
    r#"[{"role":"user","content":"hi"}]"#
)]
#[case::missing_content_kept(r#"[{"role":"user"}]"#, r#"[{"role":"user"}]"#)]
#[case::non_object_message_kept(r#"["loose"]"#, r#"["loose"]"#)]
fn audio_blocks_are_stripped_from_message_content(#[case] body: &str, #[case] expected: &str) {
    assert_eq!(strip_audio_content_blocks(&value(body)), value(expected));
}
