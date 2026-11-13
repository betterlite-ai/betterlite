use betterlite_server::mcp::outbound_credentials::OAuthTokenStore as _;
use betterlite_server::mcp::outbound_credentials::presented_store::PresentedOAuthTokenStore;
use serde_json::Value;

use super::{parse_token, runtime, value_error, view};
use crate::helpers::HelperResult;

pub(super) fn token(input: &Value) -> HelperResult {
    let minted = parse_token(input.get("token").unwrap_or(&Value::Null))?;
    let rendered = format!("{minted:?}");
    let visible = minted
        .refresh_token
        .as_ref()
        .is_some_and(|secret| rendered.contains(secret));
    let store = PresentedOAuthTokenStore {
        token: minted.clone(),
    };
    runtime()?.block_on(async {
        let fetched = store
            .fetch("user", "server")
            .await
            .map_err(|error| value_error(error.to_string()))?;
        Ok(serde_json::json!({
            "token": view(&minted),
            "access_token_in_debug": rendered.contains(&minted.access_token),
            "refresh_token_in_debug": visible,
            "has_refresh_token_marker": rendered.contains("has_refresh_token: true"),
            "presented": fetched.as_ref().map(view),
        }))
    })
}
