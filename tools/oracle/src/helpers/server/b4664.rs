mod fakes;
mod runners;
mod support;
mod tests;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::client_config::sso_client_config;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    ClientConfig(support::ClientConfigInput),
    Refresh(runners::RefreshInput),
    Store(runners::StoreInput),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ClientConfig(config) => Ok(client_config_answer(config)),
            Input::Refresh(refresh) => support::runtime()?.block_on(runners::refreshed(refresh)),
            Input::Store(store) => support::runtime()?.block_on(runners::fetched(store)),
        }
    }
}

fn client_config_answer(input: &support::ClientConfigInput) -> Value {
    let config = sso_client_config(&input.vars);
    let rendered = config.as_ref().map(|config| {
        json!({
            "token_endpoint": config.token_endpoint,
            "client_id": config.client_id,
            "auth_method": config.auth_method.as_str(),
            "client_secret_len": config.client_secret.expose_secret().len(),
        })
    });
    json!({ "config": rendered })
}
