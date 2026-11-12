use std::sync::atomic::{AtomicUsize, Ordering};

use betterlite_server::mcp::outbound_credentials::refresh_store::InProcessRefreshCoordinator;
use betterlite_server::mcp::outbound_credentials::{
    OAuthToken, RefreshCoordinator as _, RefreshStep, TokenKey, TokenStoreUnavailable,
};
use serde::Deserialize;
use tokio::task::yield_now;

use super::runtime;
use super::support::{TokenOutcome, result_view};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) refresh: TokenOutcome,
}

fn refresh_step<'call>(
    calls: &'call AtomicUsize,
    result: &'call Result<Option<OAuthToken>, TokenStoreUnavailable>,
) -> RefreshStep<'call, Option<OAuthToken>, TokenStoreUnavailable> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            yield_now().await;
            result.clone()
        })
    })
}

fn reread_step(calls: &AtomicUsize) -> RefreshStep<'_, Option<OAuthToken>, TokenStoreUnavailable> {
    Box::new(move || {
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
    })
}

pub(super) fn run(input: &Input) -> HelperResult {
    let result = input.refresh.clone().resolve()?;
    let coordinator =
        InProcessRefreshCoordinator::<Option<OAuthToken>, TokenStoreUnavailable>::default();
    let refreshes = AtomicUsize::new(0);
    let rereads = AtomicUsize::new(0);
    runtime()?.block_on(async {
        let key = TokenKey {
            user_id: "user".to_owned(),
            server_id: "server".to_owned(),
        };
        let (first, second, third) = tokio::join!(
            coordinator.run(
                key.clone(),
                refresh_step(&refreshes, &result),
                reread_step(&rereads),
            ),
            coordinator.run(
                key.clone(),
                refresh_step(&refreshes, &result),
                reread_step(&rereads),
            ),
            coordinator.run(
                key,
                refresh_step(&refreshes, &result),
                reread_step(&rereads)
            ),
        );
        Ok(serde_json::json!({
            "results": [result_view(first), result_view(second), result_view(third)],
            "refresh_calls": refreshes.load(Ordering::SeqCst),
            "reread_calls": rereads.load(Ordering::SeqCst),
        }))
    })
}
