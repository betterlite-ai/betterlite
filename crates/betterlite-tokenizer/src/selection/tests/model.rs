use rstest::rstest;

use crate::selection::fix_model_name;

use super::catalog;

#[rstest]
#[case::azure_deployment_alias("gpt-35-turbo", "gpt-3.5-turbo")]
#[case::azure_sized_deployment_alias("gpt-35-turbo-16k", "gpt-3.5-turbo-16k")]
#[case::azure_instruct_deployment_alias("gpt-35-turbo-instruct", "gpt-3.5-turbo-instruct")]
#[case::azure_41_alias_without_35_stays("azure/gpt-41", "azure/gpt-41")]
#[case::catalog_openai_model_stays("gpt-4o", "gpt-4o")]
#[case::catalog_alias_of_openai_entry_stays("openai-alias", "openai-alias")]
#[case::catalog_alias_of_azure_entry_normalizes("azure-alias", "gpt-3.5-turbo")]
#[case::provider_prefixed_azure_model_is_not_openai("azure/gpt-4o", "gpt-3.5-turbo")]
#[case::unknown_model_takes_the_default("some-unknown-model", "gpt-3.5-turbo")]
#[case::single_colon_finetune_key_is_excluded("ft:gpt-4o-2024-08-06", "gpt-3.5-turbo")]
#[case::multicolumn_finetune_key_stays_in_the_openai_set("ft:gpt-4o:org", "ft:gpt-4o:org")]
fn token_formatting_behavior_tests_behavior(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(fix_model_name(model, &catalog()), expected);
}
