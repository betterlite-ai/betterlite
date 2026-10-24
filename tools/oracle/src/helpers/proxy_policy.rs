use std::collections::BTreeMap;

use betterlite_config::proxy::{ProxyPolicy, ProxyRoute};
use betterlite_config::types::LiteLlmSettings;
use reqwest::Url;
use secrecy::ExposeSecret as _;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProxyPolicyInput {
    pub(crate) url: String,
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) aiohttp_trust_env: bool,
    pub(crate) disable_aiohttp_trust_env: bool,
}

pub(crate) fn invoke(
    input: &ProxyPolicyInput,
) -> Result<serde_json::Value, Box<super::HelperError>> {
    let settings = LiteLlmSettings {
        num_retries: None,
        request_timeout: None,
        drop_params: None,
        aiohttp_trust_env: input.aiohttp_trust_env,
        disable_aiohttp_trust_env: input.disable_aiohttp_trust_env,
    };
    let policy = ProxyPolicy::resolve(&settings, &input.environment);
    let url = Url::parse(&input.url).map_err(|error| Box::new(helper_error(error.to_string())))?;
    let message = match policy.route(&url) {
        None => return Ok(serde_json::json!({"proxy": null})),
        Some(ProxyRoute::Forward(server)) => {
            let credentials = server
                .credentials
                .as_ref()
                .map(|found| format!("{}:{}", found.username, found.password.expose_secret()));
            let proxy = serde_json::json!({
                "scheme": "http",
                "host": server.host,
                "port": server.port,
                "credentials": credentials,
            });
            return Ok(serde_json::json!({ "proxy": proxy }));
        }
        Some(ProxyRoute::Unsupported { scheme }) => format!("{scheme} proxies are not supported"),
        Some(ProxyRoute::Invalid { name }) => format!("the {name} proxy URL is not valid"),
    };
    Err(Box::new(helper_error(message)))
}

fn helper_error(message: String) -> super::HelperError {
    super::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    }
}
