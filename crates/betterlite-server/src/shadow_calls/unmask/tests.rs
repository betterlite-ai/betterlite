#![cfg(test)]

use rstest::rstest;

use super::unmask_preference;

#[rstest]
#[case::a_real_first("A", true, "real")]
#[case::a_shadow_first("A", false, "shadow")]
#[case::b_real_first("B", true, "shadow")]
#[case::b_shadow_first("B", false, "real")]
#[case::lowercase_a_real_first("a", true, "real")]
#[case::lowercase_b_shadow_first("b", false, "real")]
#[case::padded_a_shadow_first(" a ", false, "shadow")]
#[case::padded_b_real_first(" B\n", true, "shadow")]
#[case::tie("tie", true, "tie")]
#[case::uppercase_tie("TIE", false, "tie")]
#[case::another_word("preference-a", true, "tie")]
#[case::empty("", false, "tie")]
fn unmask_maps_the_blind_letter_back_to_the_arm(
    #[case] raw: &str,
    #[case] real_is_a: bool,
    #[case] expected: &str,
) {
    assert_eq!(unmask_preference(raw, real_is_a), expected);
}
