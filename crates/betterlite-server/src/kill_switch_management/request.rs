use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::agents::json::AgentJson;
use betterlite_core::agents::{AgentKillSwitchAuth, AgentKillSwitchConfig, AgentKillSwitchMethod};
use betterlite_http::user_url::error::InvalidUrl;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

use super::query::merged_query;

#[derive(Clone, Debug)]
pub struct KillSwitchRequest {
    pub method: AgentKillSwitchMethod,
    pub url: String,
    pub reported_url: String,
    pub headers: BTreeMap<String, SecretString>,
    pub body: Option<BTreeMap<String, AgentJson>>,
}

#[derive(Debug, Error)]
pub enum KillSwitchRequestError {
    #[error("the kill switch URL is not valid: {0}")]
    Url(#[from] InvalidUrl),
}

pub fn build_kill_switch_request(
    config: &AgentKillSwitchConfig,
) -> Result<KillSwitchRequest, KillSwitchRequestError> {
    let mut url = HttpxUrl::parse(&config.url)?;
    url.query = merged_query(url.query.as_deref(), &config.query_params);
    let mut reported = url.clone();
    reported.query = None;
    let mut headers = BTreeMap::new();
    for (name, value) in &config.headers {
        headers.insert(name.clone(), SecretString::from(value.as_str()));
    }
    add_auth_headers(&mut headers, config.auth.as_ref());
    Ok(KillSwitchRequest {
        method: config.method.clone(),
        url: url.href(),
        reported_url: reported.href(),
        headers,
        body: config.body.clone(),
    })
}

#[must_use]
pub const fn method_text(method: &AgentKillSwitchMethod) -> &'static str {
    match method {
        AgentKillSwitchMethod::Post => "POST",
        AgentKillSwitchMethod::Put => "PUT",
        AgentKillSwitchMethod::Patch => "PATCH",
        AgentKillSwitchMethod::Delete => "DELETE",
        AgentKillSwitchMethod::Get => "GET",
    }
}

fn add_auth_headers(
    headers: &mut BTreeMap<String, SecretString>,
    auth: Option<&AgentKillSwitchAuth>,
) {
    match auth {
        None => {}
        Some(AgentKillSwitchAuth::Bearer { token }) => {
            let secret = token.expose_secret();
            let value = format!("Bearer {secret}");
            insert_auth_header(
                headers,
                String::from("Authorization"),
                SecretString::from(value),
            );
        }
        Some(AgentKillSwitchAuth::ApiKey {
            header_name,
            api_key,
        }) => {
            insert_auth_header(headers, header_name.0.clone(), api_key.clone());
        }
        Some(AgentKillSwitchAuth::Basic { username, password }) => {
            let secret = password.expose_secret();
            let credentials = STANDARD.encode(format!("{username}:{secret}"));
            let value = format!("Basic {credentials}");
            insert_auth_header(
                headers,
                String::from("Authorization"),
                SecretString::from(value),
            );
        }
    }
}

fn insert_auth_header(
    headers: &mut BTreeMap<String, SecretString>,
    name: String,
    value: SecretString,
) {
    // HTTP header names are case-insensitive, so an auth header must replace a configured one whatever its case
    headers.retain(|existing, _| !existing.eq_ignore_ascii_case(&name));
    headers.insert(name, value);
}

mod tests;
