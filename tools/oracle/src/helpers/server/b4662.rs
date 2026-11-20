mod auth;
mod cache;
mod flow;
mod issuer;
mod support;

use betterlite_mcp::server_record::MCPServer;
use betterlite_server::mcp::oauth_caches::compute_per_user_token_ttl;
use serde::Deserialize;
use serde_json::json;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Identity(support::ServersInput),
    Oauth2GetToken(cache::GetTokenInput),
    Oauth2Fetch(cache::FetchInput),
    Oauth2Invalidate(cache::InvalidateInput),
    PerUserGet(cache::PerUserInput),
    PerUserSet(cache::PerUserInput),
    PerUserDelete(cache::PerUserInput),
    PerUserTtl(cache::PerUserTtlInput),
    FlowClassify(flow::ClassifyInput),
    FlowBackfill(flow::FlowBackfillInput),
    IssuerBackfill(issuer::IssuerBackfillInput),
    AuthResolve(auth::ResolveInput),
    TokenHeader(auth::HeaderInput),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Identity(value) => cache::identity(value),
            Input::Oauth2GetToken(value) => support::block_on(cache::get_token(value)),
            Input::Oauth2Fetch(value) => support::block_on(cache::fetch(value)),
            Input::Oauth2Invalidate(value) => support::block_on(cache::invalidate(value)),
            Input::PerUserGet(value) => support::block_on(cache::per_user_get(value)),
            Input::PerUserSet(value) => support::block_on(cache::per_user_set(value)),
            Input::PerUserDelete(value) => support::block_on(cache::per_user_delete(value)),
            Input::PerUserTtl(value) => {
                let server: MCPServer = serde_json::from_value(value.server.0.clone())
                    .map_err(|error| support::value_error(error.to_string()))?;
                Ok(json!({"ttl": compute_per_user_token_ttl(&server, value.expires_in)}))
            }
            Input::FlowClassify(value) => flow::classify(value),
            Input::FlowBackfill(value) => support::block_on(flow::flow_backfill(value)),
            Input::IssuerBackfill(value) => support::block_on(issuer::issuer_backfill(value)),
            Input::AuthResolve(value) => support::block_on(auth::resolve(value)),
            Input::TokenHeader(value) => auth::header(value),
        }
    }
}
