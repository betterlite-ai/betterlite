use betterlite_server::mcp::outbound_credentials::client_credentials::grant::prepare_grant;
use serde_json::{Value, json};

use super::support::{ConfigSpec, config_from, pairs};
use crate::helpers::{HelperError, HelperResult};

pub(super) fn run(configs: &[ConfigSpec]) -> HelperResult {
    let grants = configs
        .iter()
        .map(grant_view)
        .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
    Ok(json!({"grants": grants}))
}

fn grant_view(spec: &ConfigSpec) -> Result<Value, Box<HelperError>> {
    let config = config_from(spec)?;
    Ok(match prepare_grant(&config) {
        Ok(grant) => json!({
            "error": Value::Null,
            "token_url": grant.token_url,
            "form": pairs(&grant.form),
            "headers": pairs(&grant.headers),
            "identity_key": grant.identity_key,
        }),
        Err(error) => json!({
            "error": error.to_string(),
            "token_url": Value::Null,
            "form": Vec::<[String; 2]>::new(),
            "headers": Vec::<[String; 2]>::new(),
            "identity_key": Value::Null,
        }),
    })
}
