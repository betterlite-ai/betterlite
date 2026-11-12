use betterlite_server::mcp::outbound_credentials::OAuthTokenStore as _;
use betterlite_server::mcp::outbound_credentials::presented_store::PresentedOAuthTokenStore;
use serde::Deserialize;

use super::runtime;
use super::support::{TokenInput, value_error, view};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) token: TokenInput,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let token = input.token.to_token()?;
    let rendered = format!("{token:?}");
    let refresh_visible = token
        .refresh_token
        .as_ref()
        .is_some_and(|secret| rendered.contains(secret));
    let store = PresentedOAuthTokenStore {
        token: token.clone(),
    };
    runtime()?.block_on(async {
        let fetched = store
            .fetch("user", "server")
            .await
            .map_err(|error| value_error(error.to_string()))?;
        Ok(serde_json::json!({
            "token": view(&token),
            "access_token_in_debug": rendered.contains(&token.access_token),
            "refresh_token_in_debug": refresh_visible,
            "has_refresh_token_marker": rendered.contains("has_refresh_token: true"),
            "presented": fetched.as_ref().map(view),
        }))
    })
}
