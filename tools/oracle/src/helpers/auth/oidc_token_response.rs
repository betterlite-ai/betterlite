use std::fmt::Display;

use betterlite_auth::oidc_token_exchange::decode::decode_oidc_token_response;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let response = decode_oidc_token_response(input.body.as_bytes())
            .map_err(|error| proxy_exception(&error))?;
        serde_json::to_value(response.claims).map_err(|error| proxy_exception(&error))
    }
}

// LiteLLM raises every validation failure as a ProxyException, which carries no status_code
fn proxy_exception<M: Display>(message: M) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ProxyException".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
