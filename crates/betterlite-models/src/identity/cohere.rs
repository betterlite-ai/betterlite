use crate::catalog::ModelCatalog;

#[must_use]
pub fn cohere_chat_provider(
    model: &str,
    custom_llm_provider: Option<&str>,
    catalog: &ModelCatalog,
) -> (String, Option<String>) {
    let is_chat_model = |candidate: &str| {
        catalog
            .metadata(candidate)
            .is_some_and(|projection| projection.provider == "cohere_chat")
    };
    if custom_llm_provider == Some("cohere") && is_chat_model(model) {
        return (model.to_owned(), Some("cohere_chat".to_owned()));
    }
    if let Some((prefix, rest)) = model.split_once('/')
        && prefix == "cohere"
        && is_chat_model(rest)
    {
        return (rest.to_owned(), Some("cohere_chat".to_owned()));
    }
    (model.to_owned(), custom_llm_provider.map(str::to_owned))
}

mod tests;
