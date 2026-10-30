mod tests;

use super::error::RedisSettingsError;

// litellm._redis._uses_tls: startup_nodes presence wins, since a cluster has no single url to read a scheme from.
#[must_use]
pub fn uses_tls(startup_nodes_present: bool, ssl: Option<bool>, url: Option<&str>) -> bool {
    if startup_nodes_present {
        return ssl.unwrap_or(false);
    }
    url.and_then(url_scheme).map_or_else(
        || ssl.unwrap_or(false),
        |scheme| scheme.eq_ignore_ascii_case("rediss"),
    )
}

// Mirrors urlsplit's scheme grammar: a leading ALPHA then ALPHA/DIGIT/'+'/'-'/'.', then ':'; never fails.
fn url_scheme(url: &str) -> Option<&str> {
    let (candidate, _rest) = url.split_once(':')?;
    let mut chars = candidate.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    chars
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
        .then_some(candidate)
}

// litellm._redis's credential_provider branch uses urlsplit/urlunsplit; url::Url::parse is stricter (docs/deviations/B4321.yaml).
pub fn strip_userinfo(url: &str) -> Result<String, RedisSettingsError> {
    let mut parsed =
        url::Url::parse(url).map_err(|source| RedisSettingsError::InvalidUrl { source })?;
    parsed
        .set_username("")
        .map_err(|()| RedisSettingsError::UrlCannotStripUserinfo)?;
    parsed
        .set_password(None)
        .map_err(|()| RedisSettingsError::UrlCannotStripUserinfo)?;
    Ok(parsed.to_string())
}
