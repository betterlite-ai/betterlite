use betterlite_mcp::byok_token_exchange::pkce::verify_pkce;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
pub(crate) struct Input {
    code_verifier: String,
    code_challenge: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        Ok(json!(verify_pkce(
            &input.code_verifier,
            &input.code_challenge
        )))
    }
}

mod tests;
