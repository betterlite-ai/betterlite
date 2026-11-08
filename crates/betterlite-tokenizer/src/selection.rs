use betterlite_models::catalog::ModelCatalog;

use crate::engine::tiktoken::Encoding;

mod encoding;
mod tests;

const AZURE_LLMS: [&str; 6] = [
    "gpt-35-turbo",
    "gpt-35-turbo-16k",
    "gpt-35-turbo-instruct",
    "azure/gpt-41",
    "azure/gpt-41-mini",
    "azure/gpt-41-nano",
];

#[must_use]
pub fn fix_model_name(model: &str, catalog: &ModelCatalog) -> String {
    if AZURE_LLMS.contains(&model) {
        // azure deployment names carry -35 where the openai model id has -3.5
        return model.replace("-35", "-3.5");
    }
    if is_openai_chat_model(model, catalog) {
        return model.to_owned();
    }
    "gpt-3.5-turbo".to_owned()
}

#[must_use]
pub fn uses_legacy_message_accounting(model: &str, catalog: &ModelCatalog) -> bool {
    // the legacy id is an openai chat model, so no other normalization branch can reach it
    if !is_openai_chat_model(model, catalog) {
        return false;
    }
    model == "gpt-3.5-turbo-0301"
}

#[must_use]
pub fn openai_tokenizer_encoding_name(model: &str, catalog: &ModelCatalog) -> Encoding {
    let normalized = fix_model_name(model, catalog);
    if normalized.contains("gpt-4o") {
        return Encoding::O200kBase;
    }
    encoding::tiktoken_encoding_name(&normalized).unwrap_or_else(|| {
        tracing::debug!("model {normalized} has no tiktoken encoding; using cl100k_base");
        Encoding::Cl100kBase
    })
}

fn is_openai_chat_model(model: &str, catalog: &ModelCatalog) -> bool {
    // litellm's is_openai_finetune_model keeps only keys with at most one colon out of the set
    let finetune = model.starts_with("ft:") && model.matches(':').count() <= 1;
    !finetune
        && catalog
            .metadata(model)
            .is_some_and(|projection| projection.provider == "openai")
}
