use crate::catalog::ModelCatalog;

// litellm.azure_llms and litellm.azure_embedding_models
const AZURE_RENAMES: [(&str, &str); 7] = [
    ("gpt-35-turbo", "azure/gpt-35-turbo"),
    ("gpt-35-turbo-16k", "azure/gpt-35-turbo-16k"),
    ("gpt-35-turbo-instruct", "azure/gpt-35-turbo-instruct"),
    ("azure/gpt-41", "gpt-4.1"),
    ("azure/gpt-41-mini", "gpt-4.1-mini"),
    ("azure/gpt-41-nano", "gpt-4.1-nano"),
    ("ada", "azure/ada"),
];

const VERTEX_LLAMA: &str = "vertex_ai-llama_models";
const VERTEX_VERSIONED: [&str; 2] = ["vertex_ai-mistral_models", "vertex_ai-ai21_models"];

// LiteLLM builds these sets from the catalog with "vertex_ai/" cut out of each key.
fn vertex_set_has(catalog: &ModelCatalog, providers: &[&str], name: &str) -> bool {
    catalog.lookup.iter().any(|(key, canonical)| {
        let declared = catalog
            .entries
            .get(canonical.as_ref())
            .map(|entry| entry.fields.metadata.litellm_provider.as_str());
        declared.is_some_and(|provider| providers.contains(&provider))
            && key.replace("vertex_ai/", "") == name
    })
}

// The renames _get_model_info_helper applies to the model and provider before it builds any candidate name.
#[must_use]
pub fn remap<'query>(
    catalog: &ModelCatalog,
    model: &'query str,
    provider: Option<&'query str>,
) -> (String, Option<&'query str>) {
    let renamed = AZURE_RENAMES
        .iter()
        .find_map(|(from, to)| (*from == model).then_some(*to))
        .unwrap_or(model);
    if !matches!(provider, Some("vertex_ai" | "vertex_ai_beta")) {
        return (renamed.to_owned(), provider);
    }
    let llama = format!("meta/{renamed}");
    if vertex_set_has(catalog, &[VERTEX_LLAMA], &llama) {
        return (llama, Some("vertex_ai"));
    }
    let latest = format!("{renamed}@latest");
    if vertex_set_has(catalog, &VERTEX_VERSIONED, &latest) {
        return (latest, Some("vertex_ai"));
    }
    (renamed.to_owned(), Some("vertex_ai"))
}
