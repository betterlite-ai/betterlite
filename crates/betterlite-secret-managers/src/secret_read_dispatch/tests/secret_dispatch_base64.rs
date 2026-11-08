use rstest::rstest;

use crate::secret_read_dispatch::base64_codec::is_base64;

#[rstest]
#[case::empty("", true)]
#[case::one_byte("YQ==", true)]
#[case::full_quads("YWJj", true)]
#[case::two_padding("YWJjZA==", true)]
#[case::padded_plus_slash("+/==", true)]
#[case::missing_padding("YQ", false)]
#[case::short_padding("YQ=", false)]
#[case::extra_padding("YQ===", false)]
#[case::trailing_newline("YQ==\n", false)]
#[case::embedded_space("Y Q ==", false)]
#[case::nonzero_trailing_bits("YR==", false)]
#[case::url_safe_alphabet("_-==", false)]
#[case::only_padding("====", false)]
fn the_roundtrip_decides_what_is_base64(#[case] value: &str, #[case] expected: bool) {
    assert_eq!(is_base64(value), expected);
}
