#![cfg(test)]

use rstest::rstest;

use super::{decode_prompt_id, encode_prompt_id};

#[rstest]
#[case::nested("invoice/extract", "gitlab::invoice::extract")]
#[case::single("greeting", "gitlab::greeting")]
#[case::deep("a/b/c", "gitlab::a::b::c")]
#[case::already_encoded("gitlab::invoice::extract", "gitlab::invoice::extract")]
#[case::prefixed_slashes_stay("gitlab::a/b", "gitlab::a/b")]
#[case::empty("", "gitlab::")]
fn encode_prompt_id_scopes_a_path_and_leaves_encoded_ids_alone(
    #[case] raw: &str,
    #[case] expected: &str,
) {
    assert_eq!(encode_prompt_id(raw), expected);
}

#[rstest]
#[case::nested("gitlab::invoice::extract", "invoice/extract")]
#[case::single("gitlab::greeting", "greeting")]
#[case::prefix_only("gitlab::", "")]
#[case::unencoded_path("invoice/extract", "invoice/extract")]
#[case::separators_without_prefix_stay("a::b", "a::b")]
#[case::mixed("gitlab::a/b::c", "a/b/c")]
fn decode_prompt_id_restores_a_path_and_leaves_plain_paths_alone(
    #[case] encoded: &str,
    #[case] expected: &str,
) {
    assert_eq!(decode_prompt_id(encoded), expected);
}

#[test]
fn a_path_survives_an_encode_decode_round_trip() {
    let encoded = encode_prompt_id("chat/greeting");
    assert_eq!(decode_prompt_id(&encoded), "chat/greeting");
    assert_eq!(encode_prompt_id(&encoded), encoded);
}
