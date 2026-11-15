use crate::databricks_auth::user_agent::build_user_agent;
use rstest::rstest;

#[rstest]
#[case::absent(None, "litellm/1.105.1")]
#[case::bare_partner(Some("acme"), "acme_litellm/1.105.1")]
#[case::version_ignored(Some("mycompany/1.0.0"), "mycompany_litellm/1.105.1")]
#[case::spaced(Some("  partner_product/2.0.0  "), "partner_product_litellm/1.105.1")]
#[case::underscores(Some("a_1-b_2"), "a_1-b_2_litellm/1.105.1")]
#[case::trailing_slash(Some("acme/"), "acme_litellm/1.105.1")]
#[case::only_underscores(Some("___"), "litellm/1.105.1")]
#[case::only_hyphen(Some("-"), "litellm/1.105.1")]
#[case::space_inside(Some("a b"), "litellm/1.105.1")]
#[case::empty(Some(""), "litellm/1.105.1")]
fn a_partner_needs_a_letter_outside_its_separators(
    #[case] custom: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(build_user_agent(custom, "1.105.1"), expected);
}
