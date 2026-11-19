mod auth;
mod backfill;
mod cache;
mod support;

use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Identity(support::ServersInput),
    Oauth2GetToken(cache::GetTokenInput),
    Oauth2Fetch(cache::FetchInput),
    Oauth2Invalidate(cache::InvalidateInput),
    PerUserGet(cache::PerUserGetInput),
    PerUserSet(cache::PerUserSetInput),
    PerUserDelete(cache::PerUserDeleteInput),
    PerUserTtl(cache::PerUserTtlInput),
    FlowClassify(backfill::ClassifyInput),
    FlowBackfill(backfill::FlowBackfillInput),
    IssuerBackfill(backfill::IssuerBackfillInput),
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
            Input::PerUserTtl(value) => cache::per_user_ttl(value),
            Input::FlowClassify(value) => backfill::classify(value),
            Input::FlowBackfill(value) => support::block_on(backfill::flow_backfill(value)),
            Input::IssuerBackfill(value) => support::block_on(backfill::issuer_backfill(value)),
            Input::AuthResolve(value) => support::block_on(auth::resolve(value)),
            Input::TokenHeader(value) => auth::header(value),
        }
    }
}
