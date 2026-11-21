use std::error::Error as StdError;
use std::future::Future;

use betterlite_config::sso_identity_bridge::SsoIdentityBridgeSettings;
use betterlite_db::sso_assertion_rows::AssertionRows;
use betterlite_mcp::sso_assertions::assertion::SsoIdentityAssertion;
use betterlite_mcp::sso_assertions::store::{DbSsoAssertionStore, SsoAssertionStoreError};

use super::provider::id_jag_assertion_capture_gap;

const UNCAPTURED_WARNING: &str =
    "SSO login captured no IdP identity assertion while an oauth2_id_jag MCP server is registered";
const NO_ID_TOKEN_DETAIL: &str =
    "the identity provider's token response carried no usable id_token";
const DEBUG_CALLBACK_WARNING: &str = "SSO debug callback ran with an oauth2_id_jag capture gap";
const LOGIN_RETENTION_DEBUG: &str = "Could not check for oauth2_id_jag MCP servers after SSO login";
const DEBUG_RETENTION_DEBUG: &str = "Could not check for oauth2_id_jag MCP servers";

pub trait RetentionCheck {
    type Error: StdError + Send + Sync + 'static;

    fn retention_enabled(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send;
}

impl<R> RetentionCheck for DbSsoAssertionStore<R>
where
    R: AssertionRows + Send + Sync,
{
    type Error = SsoAssertionStoreError<R::Error>;

    fn retention_enabled(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send {
        Self::retention_enabled(self)
    }
}

// ast-grep-ignore: no-long-names
pub async fn warn_if_id_jag_assertion_uncaptured<C>(
    settings: &SsoIdentityBridgeSettings,
    assertion: Option<&SsoIdentityAssertion>,
    retention: &C,
) where
    C: RetentionCheck + Sync,
{
    if assertion.is_some() {
        return;
    }
    // diagnostics only: a failing retention query writes a debug line and never fails the login
    let enabled = match retention.retention_enabled().await {
        Ok(enabled) => enabled,
        Err(error) => {
            tracing::debug!("{LOGIN_RETENTION_DEBUG}: {error}");
            return;
        }
    };
    if !enabled {
        return;
    }
    let detail =
        id_jag_assertion_capture_gap(settings).unwrap_or_else(|| String::from(NO_ID_TOKEN_DETAIL));
    tracing::warn!("{UNCAPTURED_WARNING}: {detail}");
}

// the page's own diagnostic: it names the gap before asking the retention gate, so generic OIDC never queries
pub async fn warn_if_id_jag_capture_gap<C>(settings: &SsoIdentityBridgeSettings, retention: &C)
where
    C: RetentionCheck + Sync,
{
    let Some(gap) = id_jag_assertion_capture_gap(settings) else {
        return;
    };
    let enabled = match retention.retention_enabled().await {
        Ok(enabled) => enabled,
        Err(error) => {
            tracing::debug!("{DEBUG_RETENTION_DEBUG}: {error}");
            return;
        }
    };
    if !enabled {
        return;
    }
    tracing::warn!("{DEBUG_CALLBACK_WARNING}: {gap}");
}
