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

// A requested param is copied only when the class's narrowed supported list names it.
pub(super) fn map_supported(supported: &[&str], requested: &Object, existing: &mut Object) {
    for (name, value) in requested {
        if supported.contains(&name.as_str()) {
            existing.insert(name.clone(), value.clone());
        }
    }
}

// The alias overwrites a caller's max_tokens, and the inherited mapping never sees the old key.
pub(super) fn with_token_alias(requested: &Object) -> Object {
    let mut aliased = requested.clone();
    if let Some(limit) = aliased.remove("max_completion_tokens") {
        aliased.insert(String::from("max_tokens"), limit);
    }
    aliased
}
