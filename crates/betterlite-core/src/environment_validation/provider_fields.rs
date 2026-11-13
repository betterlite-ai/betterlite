#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderFieldSet {
    Databricks,
    Ollama,
    AzureAi,
}

// The field lists sit in the provider crates above this one, so this names which list applies.
#[must_use]
pub fn get_provider_fields(custom_llm_provider: &str) -> Option<ProviderFieldSet> {
    match custom_llm_provider {
        "databricks" => Some(ProviderFieldSet::Databricks),
        "ollama" => Some(ProviderFieldSet::Ollama),
        "azure_ai" => Some(ProviderFieldSet::AzureAi),
        _ => None,
    }
}
