use betterlite_cache::cache_key::value::{KeyObject, KeyValue};
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::call_type::CachingSupportedCallType;
use betterlite_models::decisions::Json;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Store(StoreInput),
    Wrap(WrapInput),
}

#[derive(Deserialize)]
pub(crate) struct StoreInput {
    pub(crate) scenarios: Vec<StoreScenario>,
}

#[derive(Deserialize)]
pub(crate) struct StoreScenario {
    pub(crate) function_name: String,
    pub(crate) supported_call_types: Option<Vec<CachingSupportedCallType>>,
    pub(crate) cache_configured: bool,
    pub(crate) cache_type: LiteLlmCacheType,
    pub(crate) dual: bool,
    pub(crate) kwargs: KeyObject,
    #[serde(default)]
    pub(crate) args: Vec<KeyValue>,
    pub(crate) result: StoreResult,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum StoreResult {
    Chat(Body),
    Embedding(Body),
    Raw(Body),
}

#[derive(Deserialize)]
pub(crate) struct Body {
    pub(crate) body: Json,
}

#[derive(Deserialize)]
pub(crate) struct WrapInput {
    pub(crate) scenarios: Vec<WrapScenario>,
}

#[derive(Deserialize)]
pub(crate) struct WrapScenario {
    pub(crate) function_name: String,
    pub(crate) call_type: CachingSupportedCallType,
    pub(crate) supported_call_types: Option<Vec<CachingSupportedCallType>>,
    pub(crate) cache_configured: bool,
    pub(crate) result_kind: ResultKindInput,
    pub(crate) kwargs: KeyObject,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResultKindInput {
    AsyncStream,
    SyncStream,
    Response,
}
