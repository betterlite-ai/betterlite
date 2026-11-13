use rstest::rstest;

use crate::ptu_pricing::numeric::float_literal;

#[rstest]
#[case::integer("1", true)]
#[case::decimal("1.5", true)]
#[case::leading_point(".5", true)]
#[case::trailing_point("5.", true)]
#[case::exponent("1e3", true)]
#[case::signed_exponent("1.5e-3", true)]
#[case::underscored("1_0.5", true)]
#[case::dot_only(".", false)]
#[case::exponent_only("e5", false)]
#[case::empty("", false)]
#[case::double_dot("1.2.3", false)]
#[case::trailing_underscore("1_", false)]
#[case::non_finite("inf", false)]
fn float_literal_follows_the_float_grammar(#[case] text: &str, #[case] expected: bool) {
    assert_eq!(float_literal(text), expected);
}
