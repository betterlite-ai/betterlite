use betterlite_openai::xai_login::pkce::{build_authorize_url, pkce_pair_from};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    entropy: Vec<u8>,
    authorization_endpoint: String,
    redirect_uri: String,
    state: String,
    nonce: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let (verifier, challenge) = pkce_pair_from(&input.entropy);
        Ok(json!({
            "verifier": verifier.expose_secret(),
            "challenge": challenge,
            "url": build_authorize_url(
                &input.authorization_endpoint,
                &input.redirect_uri,
                &challenge,
                &input.state,
                &input.nonce,
            ),
        }))
    }
}
