use betterlite_openai::openai_base_config::{
    ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};

// The partner policies read the inherited GPT base list, so the config carries no family settings.
pub(super) fn base_config(family: ConfigFamily) -> OpenAiConfig {
    OpenAiConfig {
        family,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

// _map_openai_params copies a requested param only when the class's own supported list names it;
// the list is passed in because a partner narrows the inherited one before mapping.
pub(super) fn map_supported(supported: &[&str], requested: &Object, existing: &mut Object) {
    for (name, value) in requested {
        if supported.contains(&name.as_str()) {
            existing.insert(name.clone(), value.clone());
        }
    }
}

// map_openai_params renames max_completion_tokens to max_tokens in place, overwriting a max_tokens
// the caller already sent, and the inherited mapping never sees the old key.
pub(super) fn with_token_alias(requested: &Object) -> Object {
    let mut aliased = requested.clone();
    if let Some(limit) = aliased.remove("max_completion_tokens") {
        aliased.insert(String::from("max_tokens"), limit);
    }
    aliased
}
