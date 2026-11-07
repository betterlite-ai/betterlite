use std::sync::PoisonError;

use axum::http::Method;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_roi::settings::normalize_email;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

use super::client::GitHub;
use super::models::GitHubUserProfile;
use super::transport::Transport;

// Python's quote(safe="") leaves only the unreserved characters alone; "/" and everything else is encoded
const QUOTE_SAFE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'.')
    .remove(b'-')
    .remove(b'~');

pub async fn profile_email<T>(github: &GitHub<T>, login: &str, fallback: &str) -> String
where
    T: Transport,
{
    let key = caseless::default_case_fold_str(login);
    let cached = github
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        .cloned();
    if let Some(address) = cached {
        return address.unwrap_or_else(|| fallback.to_owned());
    }
    let address = load_profile_email(github, login).await;
    github
        .profiles
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, address.clone());
    address.unwrap_or_else(|| fallback.to_owned())
}

async fn load_profile_email<T>(github: &GitHub<T>, login: &str) -> Option<String>
where
    T: Transport,
{
    let encoded = utf8_percent_encode(login, QUOTE_SAFE);
    let url = github.url(&format!("users/{encoded}")).ok()?;
    let request = OutboundRequest {
        method: Method::GET,
        url,
        query: Vec::new(),
        headers: github.headers.clone(),
        body: RequestBody::Empty,
        deadline: github.timeout,
    };
    let response = github.transport.send(request).await.ok()?;
    if response.status != 200 {
        return None;
    }
    let profile: GitHubUserProfile = serde_json::from_slice(&response.body).ok()?;
    let email = profile.email.unwrap_or_default();
    Some(normalize_email(&email))
}
