use rstest::rstest;

use crate::selection::uses_legacy_message_accounting;

use super::{catalog, empty_catalog};

#[rstest]
#[case::the_0301_model_in_the_catalog_is_legacy("gpt-3.5-turbo-0301", false, true)]
#[case::the_0301_model_outside_the_catalog_normalizes_away("gpt-3.5-turbo-0301", true, false)]
#[case::the_azure_deployment_alias_is_not_legacy("gpt-35-turbo-0301", false, false)]
#[case::an_ordinary_catalog_model_is_not_legacy("gpt-4o", false, false)]
fn token_formatting_behavior_tests_behavior(
    #[case] model: &str,
    #[case] without_catalog: bool,
    #[case] expected: bool,
) {
    let models = if without_catalog {
        empty_catalog()
    } else {
        catalog()
    };
    assert_eq!(uses_legacy_message_accounting(model, &models), expected);
}
