use serde::Deserialize;

use super::{cache_backend, cached_store, dual_cache, refreshing_store, v2_store};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Input {
    Cache(cache_backend::Input),
    Cached(cached_store::Input),
    Refreshing(refreshing_store::Input),
    Dual(dual_cache::Input),
    #[serde(rename = "v2")]
    V2(v2_store::Input),
}

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::Cache(payload) => cache_backend::run(payload),
        Input::Cached(payload) => cached_store::run(payload),
        Input::Refreshing(payload) => refreshing_store::run(payload),
        Input::Dual(payload) => dual_cache::run(payload),
        Input::V2(payload) => v2_store::run(payload),
    }
}
