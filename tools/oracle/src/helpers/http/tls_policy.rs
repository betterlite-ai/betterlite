use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_http::tls::{configure_curve, parse_ca_pem};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) ca_pem: Option<String>,
    pub(crate) deployment_ssl_verify: Option<bool>,
    pub(crate) config_ssl_verify: Option<bool>,
    pub(crate) config_http2: Option<bool>,
    pub(crate) config_ssl_certificate: Option<String>,
    pub(crate) environment: BTreeMap<String, String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let policy = resolve_policy(input)?;
        let (_provider, curve_fallback) =
            configure_curve(&policy).map_err(|error| crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })?;
        let ssl_verify = match policy.verification {
            Verification::Enabled => serde_json::Value::Bool(true),
            Verification::Disabled => serde_json::Value::Bool(false),
            Verification::CaFile(path) => serde_json::Value::String(path),
        };
        Ok(serde_json::json!({
            "ssl_verify": ssl_verify,
            "ca_valid": input.ca_pem.as_deref().map(|pem| parse_ca_pem(pem.as_bytes()).is_ok()),
            "http2": policy.http2,
            "user_agent": policy.user_agent,
            "certificate": policy.certificate,
            "ca_file": policy.ca_file,
            "ecdh_curve": policy.ecdh_curve,
            "curve_fallback": curve_fallback,
        }))
    }
}

fn resolve_policy(input: &Input) -> Result<TlsPolicy, Box<crate::helpers::HelperError>> {
    let config_yaml = config_yaml(input);
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(&config_yaml)
    .map_err(|error| crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })?;
    TlsPolicy::resolve(&config, &input.environment, input.deployment_ssl_verify).map_err(|error| {
        Box::new(crate::helpers::HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    })
}

fn config_yaml(input: &Input) -> String {
    let mut settings = Vec::new();
    if let Some(value) = input.config_ssl_verify {
        settings.push(format!("  ssl_verify: {value}"));
    }
    if let Some(value) = input.config_http2 {
        settings.push(format!("  http2: {value}"));
    }
    if let Some(value) = &input.config_ssl_certificate {
        settings.push(format!("  ssl_certificate: {}", serde_json::json!(value)));
    }
    if settings.is_empty() {
        String::new()
    } else {
        format!("litellm_settings:\n{}\n", settings.join("\n"))
    }
}

mod tests;
