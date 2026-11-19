use secrecy::{ExposeSecret as _, SecretString};

#[derive(Clone, Copy)]
pub struct KeySources<'request> {
    pub explicit: Option<&'request SecretString>,
    pub environment: Option<&'request SecretString>,
    pub global: Option<&'request SecretString>,
}

#[derive(Clone, Copy)]
pub struct BaseSources<'request> {
    pub explicit: Option<&'request str>,
    pub environment: Option<&'request str>,
    pub fallback_environment: Option<&'request str>,
}

// LiteLLM chains the sources with `or`, so an empty explicit or environment value falls through
#[must_use]
pub fn resolve_key(sources: KeySources<'_>) -> Option<SecretString> {
    let resolved = sources
        .explicit
        .into_iter()
        .chain(sources.environment)
        .find(|key| !key.expose_secret().is_empty())
        .or(sources.global);
    resolved.map(|key| SecretString::from(key.expose_secret().to_owned()))
}

#[must_use]
pub fn resolve_base(sources: BaseSources<'_>, default: &str) -> String {
    sources
        .explicit
        .into_iter()
        .chain(sources.environment)
        .chain(sources.fallback_environment)
        .find(|value| !value.is_empty())
        .unwrap_or(default)
        .to_owned()
}
