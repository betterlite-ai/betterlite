use betterlite_config::tls::Verification;
use betterlite_http::user_url::address::is_blocked_text;
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::validate::{SystemResolver, UrlValidator};
use serde::Deserialize;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub(crate) enum Input {
    ValidateUrl {
        url: String,
        #[serde(default)]
        user_url_allowed_hosts: Vec<String>,
        ssl_verify: bool,
    },
    BlockedIp {
        addresses: Vec<String>,
    },
}

fn failure(class: &str, message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::BlockedIp { addresses } => Ok(serde_json::Value::Array(
                addresses
                    .iter()
                    .map(|address| serde_json::Value::Bool(is_blocked_text(address)))
                    .collect(),
            )),
            Input::ValidateUrl {
                url,
                user_url_allowed_hosts,
                ssl_verify,
            } => {
                let verification = if *ssl_verify {
                    Verification::Enabled
                } else {
                    Verification::Disabled
                };
                let validator =
                    UrlValidator::new(SystemResolver, user_url_allowed_hosts, &verification);
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| failure("RuntimeError", error.to_string()))?;
                let validated = runtime
                    .block_on(validator.validate_url(url))
                    .map_err(|error| {
                        let ssrf = matches!(error, UserUrlError::Ssrf(_));
                        failure(
                            if ssrf { "SSRFError" } else { "ValueError" },
                            error.to_string(),
                        )
                    })?;
                Ok(serde_json::json!({
                    "url": validated.url,
                    "host_header": validated.host_header,
                }))
            }
        }
    }
}
