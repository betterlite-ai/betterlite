use crate::provider_defaults_b::environment::EnvironmentLookup;
use crate::provider_defaults_b::globals::GlobalCredentials;

const MISTRAL_DEFAULT_BASE: &str = "https://api.mistral.ai/v1";
const XAI_DEFAULT_BASE: &str = "https://api.x.ai/v1";
const INCEPTION_DEFAULT_BASE: &str = "https://api.inceptionlabs.ai/v1";
const LEMONADE_DEFAULT_BASE: &str = "http://localhost:8000/api/v1";
const LEMONADE_DEFAULT_KEY: &str = "lemonade";

pub fn resolve_mistral<E>(
    api_base: Option<&str>,
    api_key: Option<&str>,
    env: &E,
) -> (Option<String>, Option<String>)
where
    E: EnvironmentLookup,
{
    let mut base = api_base
        .map(ToOwned::to_owned)
        .or_else(|| env.lookup("MISTRAL_AZURE_API_BASE"))
        .unwrap_or_else(|| MISTRAL_DEFAULT_BASE.to_owned());
    // Mistral's endpoint always needs a /v1 suffix, even for a custom base.
    if !base.ends_with("/v1") {
        base.push_str("/v1");
    }
    let key = api_key
        .map(ToOwned::to_owned)
        .or_else(|| env.lookup("MISTRAL_AZURE_API_KEY"))
        .or_else(|| env.lookup("MISTRAL_API_KEY"));
    (Some(base), key)
}

pub fn resolve_xai<E>(
    api_base: Option<&str>,
    api_key: Option<&str>,
    globals: &GlobalCredentials,
    env: &E,
) -> (Option<String>, Option<String>)
where
    E: EnvironmentLookup,
{
    let base = Some(
        api_base
            .map(ToOwned::to_owned)
            .or_else(|| env.lookup("XAI_API_BASE"))
            .unwrap_or_else(|| XAI_DEFAULT_BASE.to_owned()),
    );
    let key = api_key
        .map(ToOwned::to_owned)
        .or_else(|| globals.xai_key.clone())
        .or_else(|| env.lookup("XAI_API_KEY"));
    (base, key)
}

pub fn resolve_inception<E>(
    api_base: Option<&str>,
    api_key: Option<&str>,
    globals: &GlobalCredentials,
    env: &E,
) -> (Option<String>, Option<String>)
where
    E: EnvironmentLookup,
{
    let base = Some(
        api_base
            .map(ToOwned::to_owned)
            .or_else(|| env.lookup("INCEPTION_API_BASE"))
            .unwrap_or_else(|| INCEPTION_DEFAULT_BASE.to_owned()),
    );
    // An explicit base without an explicit key skips every fallback, matching litellm's guard.
    let key = if api_base.is_none() || api_key.is_some() {
        api_key
            .map(ToOwned::to_owned)
            .or_else(|| globals.inception_key.clone())
            .or_else(|| env.lookup("INCEPTION_API_KEY"))
    } else {
        None
    };
    (base, key)
}

pub fn resolve_lemonade<E>(
    api_base: Option<&str>,
    api_key: Option<&str>,
    globals: &GlobalCredentials,
    env: &E,
) -> (Option<String>, Option<String>)
where
    E: EnvironmentLookup,
{
    let base = Some(
        api_base
            .map(ToOwned::to_owned)
            .or_else(|| env.lookup("LEMONADE_API_BASE"))
            .unwrap_or_else(|| LEMONADE_DEFAULT_BASE.to_owned()),
    );
    // Lemonade falls back to its own literal key in both branches, so the key is never absent.
    let key = if api_base.is_none() || api_key.is_some() {
        api_key
            .map(ToOwned::to_owned)
            .or_else(|| globals.lemonade_key.clone())
            .or_else(|| env.lookup("LEMONADE_API_KEY"))
            .unwrap_or_else(|| LEMONADE_DEFAULT_KEY.to_owned())
    } else {
        LEMONADE_DEFAULT_KEY.to_owned()
    };
    (base, Some(key))
}

pub fn resolve_ragflow<E>(
    model: &str,
    api_base: Option<&str>,
    api_key: Option<&str>,
    globals: &GlobalCredentials,
    env: &E,
) -> (String, Option<String>, Option<String>)
where
    E: EnvironmentLookup,
{
    let base = api_base
        .map(ToOwned::to_owned)
        .or_else(|| globals.api_base.clone())
        .or_else(|| env.lookup("RAGFLOW_API_BASE"));
    let key = api_key
        .map(ToOwned::to_owned)
        .or_else(|| {
            globals
                .api_key
                .as_ref()
                .map(|key| secrecy::ExposeSecret::expose_secret(key).to_owned())
        })
        .or_else(|| env.lookup("RAGFLOW_API_KEY"));
    (format!("ragflow/{model}"), base, key)
}
