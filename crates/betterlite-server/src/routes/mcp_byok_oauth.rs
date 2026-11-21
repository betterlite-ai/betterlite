#![cfg_attr(
    not(test),
    expect(dead_code, reason = "B8624 mounts the byok OAuth routes")
)]

mod metadata;
mod tests;
mod token;

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Router;
use axum::extract::Extension;
use axum::routing::{get, post};
use betterlite_mcp::byok_token_exchange::ports::{AuthCodeLookup, ByokCredentialSink};
use betterlite_mcp::oauth_resource::challenge::BYOK_RESOURCE_METADATA_PATH;
use secrecy::SecretString;

use self::metadata::RootPrefix;

pub(crate) const AUTHORIZATION_SERVER_PATH: &str = "/.well-known/oauth-authorization-server";
pub(crate) const TOKEN_PATH: &str = "/v1/mcp/oauth/token";

// B8624 implements this over get_request_base_url, the normalized PROXY_BASE_URL and the trusted-proxy decision.
pub(crate) trait RequestBaseUrl: Send + Sync {
    fn base_url(&self, parts: &axum::http::request::Parts) -> String;
}

pub(crate) struct ByokOauthRoutes<C, S, B> {
    pub(crate) codes: C,
    pub(crate) credentials: Option<S>,
    pub(crate) master_key: Option<SecretString>,
    pub(crate) base_url: B,
    pub(crate) root_prefixes: Vec<String>,
}

pub(crate) fn routes<C, S, B>(
    state: Arc<ByokOauthRoutes<C, S, B>>,
    well_known_suffix: &str,
) -> Router
where
    C: AuthCodeLookup + 'static,
    S: ByokCredentialSink + 'static,
    B: RequestBaseUrl + 'static,
{
    let issuer_path = format!("{AUTHORIZATION_SERVER_PATH}{well_known_suffix}/v1/mcp/oauth");
    let mut router = Router::new()
        .route(
            AUTHORIZATION_SERVER_PATH,
            get(metadata::bare_document::<C, S, B>),
        )
        .route(&issuer_path, get(metadata::issuer_document::<C, S, B>))
        .route(
            BYOK_RESOURCE_METADATA_PATH,
            get(metadata::protected_resource::<C, S, B>),
        )
        .route(TOKEN_PATH, post(token::token_endpoint::<C, S, B>));
    // axum panics on a duplicate path, and LiteLLM's earlier suffix route wins over the catch-all.
    let prefixes: BTreeSet<&str> = state
        .root_prefixes
        .iter()
        .map(String::as_str)
        .filter(|prefix| *prefix != well_known_suffix)
        .collect();
    for prefix in prefixes {
        let path = format!("{AUTHORIZATION_SERVER_PATH}{prefix}/v1/mcp/oauth");
        router = router.route(
            &path,
            get(metadata::prefixed_document::<C, S, B>)
                .layer(Extension(RootPrefix(prefix.to_owned()))),
        );
    }
    router.with_state(state)
}
