#[must_use]
pub fn anthropic_text_provider(
    model: &str,
    custom_llm_provider: Option<&str>,
) -> (String, Option<String>) {
    let is_text_model =
        |candidate: &str| candidate == "claude-2" || candidate == "claude-instant-1";
    if custom_llm_provider == Some("anthropic") && is_text_model(model) {
        return (model.to_owned(), Some("anthropic_text".to_owned()));
    }
    if let Some((prefix, rest)) = model.split_once('/')
        && prefix == "anthropic"
        && is_text_model(rest)
    {
        return (rest.to_owned(), Some("anthropic_text".to_owned()));
    }
    (model.to_owned(), custom_llm_provider.map(str::to_owned))
}

mod tests;
