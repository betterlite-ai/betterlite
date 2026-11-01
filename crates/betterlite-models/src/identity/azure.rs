use crate::catalog::ModelCatalog;

// mistral_chat_models keys carry the catalog's own mistral/ prefix; cohere_chat_models keys don't.
#[must_use]
pub fn is_non_openai_azure_model(model: &str, catalog: &ModelCatalog) -> bool {
    let Some((_, model_name)) = model.split_once('/') else {
        return false;
    };
    let provider_is = |name: &str, provider: &str| {
        catalog
            .metadata(name)
            .is_some_and(|projection| projection.provider == provider)
    };
    if provider_is(model_name, "cohere_chat") {
        return true;
    }
    let mistral_key = format!("mistral/{model_name}");
    provider_is(&mistral_key, "mistral")
}

#[must_use]
pub fn is_azure_claude_model(model: &str) -> bool {
    for window in model.as_bytes().windows(6) {
        if window.eq_ignore_ascii_case(b"claude") {
            return true;
        }
    }
    false
}

mod tests;
