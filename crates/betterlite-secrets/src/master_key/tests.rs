#![cfg(test)]

use rstest::rstest;

use super::matches_raw;

#[rstest]
#[case::exact_match(Some("sk-master-1234"), Some("sk-master-1234"), true)]
#[case::different_value(Some("sk-master-1234"), Some("sk-master-9999"), false)]
#[case::different_length(Some("short"), Some("a-much-longer-master-key"), false)]
#[case::no_presented(None, Some("sk-master-1234"), false)]
#[case::no_master_key(Some("sk-master-1234"), None, false)]
#[case::both_absent(None, None, false)]
#[case::non_ascii_presented(Some("sk-m\u{e4}ster"), Some("sk-m\u{e4}ster"), false)]
#[case::non_ascii_master(Some("plain"), Some("pla\u{131}n"), false)]
#[case::digest_is_not_raw(Some("hashed-jwt-aaaa"), Some("sk-master-1234"), false)]
fn reports_raw_master_key_equality(
    #[case] presented: Option<&str>,
    #[case] master_key: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(matches_raw(presented, master_key), expected);
}
