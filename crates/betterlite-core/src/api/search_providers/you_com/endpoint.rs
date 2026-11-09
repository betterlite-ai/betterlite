use secrecy::{ExposeSecret as _, SecretString};

use super::{YOU_COM_API_BASE, YOU_COM_FREE_API_BASE, YouComSearch};

pub(super) fn endpoint(
    settings: &YouComSearch,
    api_base: Option<&str>,
    api_key: Option<&SecretString>,
) -> String {
    let Some(base) = api_base.or(settings.api_base.as_deref()) else {
        let resolved = api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or(settings.api_key.as_ref());
        return if resolved.is_some() {
            finish(YOU_COM_API_BASE)
        } else {
            YOU_COM_FREE_API_BASE.to_owned()
        };
    };
    finish(base)
}

fn finish(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1/search") || trimmed.ends_with("/v1/agents/search") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/v1/search")
    }
}
