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
    let suffix = surface.suffix();
    let default_url = format!("{DEFAULT_BASE}{suffix}");
    let base = [
        explicit,
        globals.api_base.as_deref(),
        environment.get(API_BASE_VARIABLE).map(String::as_str),
        environment.get(BASE_URL_VARIABLE).map(String::as_str),
    ]
    .into_iter()
    .flatten()
    .find(|value| !value.is_empty())
    .unwrap_or(&default_url);
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

fn non_empty_secret(value: Option<&SecretString>) -> Option<SecretString> {
    use secrecy::ExposeSecret as _;
    value.filter(|key| !key.expose_secret().is_empty()).cloned()
}

#[path = "base/tests.rs"]
mod tests;
