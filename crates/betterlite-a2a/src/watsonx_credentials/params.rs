use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_with::{DefaultOnNull, serde_as};
use thiserror::Error;

mod tests;

#[serde_as]
#[derive(Debug, Default, Deserialize)]
pub struct WxoLitellmParams {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub cp4d_host: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub instance_id: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub wxo_agent_id: String,
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub api_key: SecretString,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
}

#[derive(Debug)]
pub struct WxoRequestParams {
    pub cp4d_host: String,
    pub instance_id: String,
    pub wxo_agent_id: String,
    pub api_key: SecretString,
    pub username: Option<String>,
    pub auth_mode: String,
    pub thread_id: Option<String>,
}

#[derive(Debug, Error)]
#[error("'{key}' is required in litellm_params for WXO agents")]
pub struct MissingParam {
    pub key: &'static str,
}

pub fn extract_litellm_params(raw: WxoLitellmParams) -> Result<WxoRequestParams, MissingParam> {
    let required = [
        ("cp4d_host", raw.cp4d_host.is_empty()),
        ("instance_id", raw.instance_id.is_empty()),
        ("wxo_agent_id", raw.wxo_agent_id.is_empty()),
        ("api_key", raw.api_key.expose_secret().is_empty()),
    ];
    if let Some((key, _)) = required.iter().find(|(_, missing)| *missing) {
        return Err(MissingParam { key });
    }
    Ok(WxoRequestParams {
        cp4d_host: raw.cp4d_host,
        instance_id: raw.instance_id,
        wxo_agent_id: raw.wxo_agent_id,
        api_key: raw.api_key,
        // LiteLLM treats an empty string as absent for every key
        username: raw.username.filter(|text| !text.is_empty()),
        auth_mode: raw
            .auth_mode
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "cp4d".to_owned()),
        thread_id: raw.thread_id.filter(|text| !text.is_empty()),
    })
}
