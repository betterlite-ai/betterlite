use std::collections::BTreeMap;

use secrecy::SecretString;

pub const MESSAGES_SUFFIX: &str = "/v1/messages";
pub const LEGACY_SUFFIX: &str = "/v1/complete";
pub const DEFAULT_BASE: &str = "https://api.anthropic.com";
pub const API_BASE_VARIABLE: &str = "ANTHROPIC_API_BASE";
pub const BASE_URL_VARIABLE: &str = "ANTHROPIC_BASE_URL";
pub const API_KEY_VARIABLE: &str = "ANTHROPIC_API_KEY";

// main.py has one branch per Anthropic endpoint; only the suffix and its notice differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Messages,
    Legacy,
}

impl Surface {
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Messages => MESSAGES_SUFFIX,
            Self::Legacy => LEGACY_SUFFIX,
        }
    }
}

// The values LiteLLM's module globals hold when the caller passed nothing.
#[derive(Debug, Clone, Default)]
pub struct Globals {
    pub api_base: Option<String>,
    pub anthropic_key: Option<SecretString>,
    pub api_key: Option<SecretString>,
}

// The injected environment; the secret manager loaders belong to B567 and B500.
pub type Environment = BTreeMap<String, String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBase {
    pub url: String,
    pub notice: Option<String>,
}

// Explicit base, litellm.api_base, ANTHROPIC_API_BASE, ANTHROPIC_BASE_URL, then the provider default.
#[must_use]
pub fn resolve_base(
    surface: Surface,
    explicit: Option<&str>,
    globals: &Globals,
    environment: &Environment,
    disable_suffix: bool,
) -> ResolvedBase {
    let base = non_empty(explicit)
        .or_else(|| non_empty(globals.api_base.as_deref()))
        .or_else(|| environment.get(API_BASE_VARIABLE).map(String::as_str))
        .filter(|value| !value.is_empty())
        .or_else(|| environment.get(BASE_URL_VARIABLE).map(String::as_str))
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_BASE);
    let suffix = surface.suffix();
    if disable_suffix {
        return ResolvedBase {
            url: base.to_owned(),
            notice: Some(format!(
                "LITELLM_ANTHROPIC_DISABLE_URL_SUFFIX is set, skipping {suffix} suffix"
            )),
        };
    }
    if base.ends_with(suffix) {
        return ResolvedBase {
            url: base.to_owned(),
            notice: None,
        };
    }
    ResolvedBase {
        url: format!("{base}{suffix}"),
        notice: None,
    }
}

// Explicit key, litellm.anthropic_key, litellm.api_key, then the raw variable, whose empty value is kept.
#[must_use]
pub fn resolve_key(
    explicit: Option<&SecretString>,
    globals: &Globals,
    environment: &Environment,
) -> Option<SecretString> {
    non_empty_secret(explicit)
        .or_else(|| non_empty_secret(globals.anthropic_key.as_ref()))
        .or_else(|| non_empty_secret(globals.api_key.as_ref()))
        .or_else(|| {
            environment
                .get(API_KEY_VARIABLE)
                .map(|value| SecretString::from(value.clone()))
        })
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

fn non_empty_secret(value: Option<&SecretString>) -> Option<SecretString> {
    use secrecy::ExposeSecret as _;
    value.filter(|key| !key.expose_secret().is_empty()).cloned()
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret as _;

    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> Environment {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn explicit_base_wins_and_the_suffix_is_appended() {
        let resolved = resolve_base(
            Surface::Messages,
            Some("https://explicit.test"),
            &Globals::default(),
            &environment(&[(API_BASE_VARIABLE, "https://env.test")]),
            false,
        );
        assert_eq!(resolved.url, "https://explicit.test/v1/messages");
        assert_eq!(resolved.notice, None);
    }

    #[test]
    fn a_base_that_already_ends_with_the_suffix_keeps_one() {
        let resolved = resolve_base(
            Surface::Messages,
            Some("https://explicit.test/v1/messages"),
            &Globals::default(),
            &Environment::new(),
            false,
        );
        assert_eq!(resolved.url, "https://explicit.test/v1/messages");
    }

    #[test]
    fn a_slash_ended_host_still_receives_the_suffix() {
        let resolved = resolve_base(
            Surface::Legacy,
            Some("https://explicit.test/"),
            &Globals::default(),
            &Environment::new(),
            false,
        );
        assert_eq!(resolved.url, "https://explicit.test//v1/complete");
    }

    #[test]
    fn disabling_the_suffix_keeps_the_url_and_names_the_notice() {
        let resolved = resolve_base(
            Surface::Legacy,
            Some("https://explicit.test"),
            &Globals::default(),
            &Environment::new(),
            true,
        );
        assert_eq!(resolved.url, "https://explicit.test");
        assert_eq!(
            resolved.notice.as_deref(),
            Some("LITELLM_ANTHROPIC_DISABLE_URL_SUFFIX is set, skipping /v1/complete suffix")
        );
    }

    #[test]
    fn empty_environment_values_fall_through_to_the_default() {
        let resolved = resolve_base(
            Surface::Messages,
            None,
            &Globals::default(),
            &environment(&[(API_BASE_VARIABLE, ""), (BASE_URL_VARIABLE, "")]),
            false,
        );
        assert_eq!(resolved.url, "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn the_key_chain_ends_at_the_raw_variable_even_when_empty() {
        let key = resolve_key(
            None,
            &Globals::default(),
            &environment(&[(API_KEY_VARIABLE, "")]),
        )
        .expect("the raw variable keeps its empty value");
        assert_eq!(key.expose_secret(), "");
    }
}
