use std::sync::Arc;

use betterlite_server::mcp::outbound_credentials::refresh_store::{
    InProcessRefreshCoordinator, RefreshingTokenStore,
};
use betterlite_server::mcp::outbound_credentials::{
    DEFAULT_EXPIRY_SKEW, OAuthToken, OAuthTokenStore as _, TokenStoreUnavailable, WallClock,
};
use serde::Deserialize;

use super::runtime;
use super::support::{ManualClock, ScriptedRefresher, ScriptedStore, TokenOutcome, result_view};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) inner: Vec<TokenOutcome>,
    pub(super) refresher: Vec<TokenOutcome>,
    pub(super) operations: Vec<Operation>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Operation {
    pub(super) user_id: String,
    pub(super) server_id: String,
    pub(super) now_ms: i64,
}

pub(super) fn run(input: &Input) -> HelperResult {
    let clock = Arc::new(ManualClock::default());
    let setter = Arc::clone(&clock);
    let store_clock: Arc<dyn WallClock> = clock;
    let store = RefreshingTokenStore {
        inner: ScriptedStore::new(input.inner.clone())?,
        refresher: ScriptedRefresher::new(input.refresher.clone())?,
        expiry_skew: DEFAULT_EXPIRY_SKEW,
        clock: store_clock,
        coordinator:
            InProcessRefreshCoordinator::<Option<OAuthToken>, TokenStoreUnavailable>::default(),
    };
    runtime()?.block_on(async {
        let mut results = Vec::new();
        for operation in &input.operations {
            setter.set_millis(operation.now_ms)?;
            let result = store.fetch(&operation.user_id, &operation.server_id).await;
            results.push(serde_json::json!({
                "result": result_view(result),
                "inner_reads": store.inner.reads.load(std::sync::atomic::Ordering::SeqCst),
                "refresh_calls": store.refresher.calls.load(std::sync::atomic::Ordering::SeqCst),
            }));
        }
        Ok(serde_json::json!({"operations": results}))
    })
}
