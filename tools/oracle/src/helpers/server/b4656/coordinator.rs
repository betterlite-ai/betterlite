use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_server::mcp::outbound_credentials::refresh_store::InProcessRefreshCoordinator;
use betterlite_server::mcp::outbound_credentials::{
    OAuthToken, RefreshCoordinator as _, RefreshStep, TokenKey, TokenStoreUnavailable,
};
use serde_json::Value;
use tokio::task::yield_now;

use super::{StoreResult, outcome, result_view, runtime};
use crate::helpers::HelperResult;

pub(super) type Coordinator =
    InProcessRefreshCoordinator<Option<OAuthToken>, TokenStoreUnavailable>;
type Step<'call> = RefreshStep<'call, Option<OAuthToken>, TokenStoreUnavailable>;

fn refresh_step<'call>(calls: &'call AtomicUsize, result: &'call StoreResult) -> Step<'call> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            yield_now().await;
            result.clone()
        })
    })
}

fn reread_step(calls: &AtomicUsize) -> Step<'_> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    })
}

async fn coordinated_call(
    flight: &Coordinator,
    key: &TokenKey,
    refreshes: &AtomicUsize,
    rereads: &AtomicUsize,
    result: &StoreResult,
) -> StoreResult {
    flight
        .run(
            key.clone(),
            refresh_step(refreshes, result),
            reread_step(rereads),
        )
        .await
}

pub(super) fn coordinator(input: &Value) -> HelperResult {
    let result = outcome(input.get("refresh").unwrap_or(&Value::Null))?;
    let flight = Coordinator::default();
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    let key = TokenKey {
        user_id: "user".to_owned(),
        server_id: "server".to_owned(),
    };
    runtime()?.block_on(async {
        let (first, second, third) = tokio::join!(
            coordinated_call(&flight, &key, &refreshes, &rereads, &result),
            coordinated_call(&flight, &key, &refreshes, &rereads, &result),
            coordinated_call(&flight, &key, &refreshes, &rereads, &result),
        );
        Ok(serde_json::json!({
            "results": [result_view(first), result_view(second), result_view(third)],
            "refresh_calls": refreshes.load(Ordering::SeqCst),
            "reread_calls": rereads.load(Ordering::SeqCst),
        }))
    })
}
