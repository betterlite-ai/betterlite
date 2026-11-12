use rstest::rstest;

use crate::environment_validation::provider_fields::{ProviderFieldSet, get_provider_fields};

#[rstest]
#[case::databricks("databricks", Some(ProviderFieldSet::Databricks))]
#[case::ollama("ollama", Some(ProviderFieldSet::Ollama))]
#[case::azure_ai("azure_ai", Some(ProviderFieldSet::AzureAi))]
#[case::openai("openai", None)]
#[case::azure("azure", None)]
#[case::ollama_chat("ollama_chat", None)]
#[case::upper_case("Databricks", None)]
#[case::padded("ollama ", None)]
#[case::empty("", None)]
fn provider_fields_tags_three_providers_and_nothing_else(
    #[case] provider: &str,
    #[case] expected: Option<ProviderFieldSet>,
) {
    assert_eq!(get_provider_fields(provider), expected);
}
