pub mod alerts;
pub mod cache;
mod key;
mod objects;
mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::auth_cache::keys::CacheKey;
use betterlite_auth::auth_cache::payload::PayloadError;
use betterlite_auth::resolved_key::auth::{AuthError, ParentOtelSpan};
use betterlite_cache::batch_reads::BatchReadsError;
use betterlite_db::budget_reset_policy::cache_invalidation::GLOBAL_PROXY_SPEND_CACHE_KEY;
use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::lock;
use betterlite_redis::request_batch::scope::ACTIVE_REQUEST_BATCHES;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::warn;

use crate::failure_hook::ports::Clock;
use crate::proxy_helpers::projected::ProjectionError;

use self::alerts::SpendBudgetAlerts;
use self::cache::{ArmedSpendRead, SpendCache};
use self::key::update_key_spend;
use self::objects::update_objects;

const UPDATE_CACHE_READ_SLOT: &str = "update_cache_read";

#[derive(Debug, thiserror::Error)]
pub enum SpendCacheUpdateError {
    #[error("the cached spend objects could not be read")]
    CacheRead,
    #[error("the request batch read could not be declared")]
    Declare(#[source] BatchReadsError),
    #[error("the request batch read could not be resolved")]
    Resolve(#[source] BatchReadsError),
    #[error("the cached api key could not be read")]
    KeyAuth(#[source] AuthError),
    #[error("the cached api key could not be decoded")]
    KeyView(#[source] serde_json::Error),
    #[error("the cached spend object could not be decoded")]
    Decode(#[source] PayloadError),
    #[error("the updated spend object could not be encoded")]
    Encode(#[source] PayloadError),
    #[error("the spend arithmetic overflowed")]
    SpendOverflow,
    #[error("the global proxy spend is not a decimal number")]
    GlobalSpend(#[source] rust_decimal::Error),
    #[error("the global proxy spend cannot be written as a json number")]
    GlobalNumber(#[source] serde_json::Error),
    #[error("the projected spend could not be computed")]
    Projection(#[source] ProjectionError),
}

pub struct UpdateDeps {
    pub cache: Arc<dyn SpendCache>,
    pub alerts: Arc<dyn SpendBudgetAlerts>,
    pub tasks: TaskTracker,
    pub clock: Arc<dyn Clock>,
}

// no Debug: a token that reaches here can still be a plaintext sk- key
#[derive(Clone, Default)]
pub struct UpdateInput {
    pub token: Option<String>,
    pub user_id: Option<String>,
    pub end_user_id: Option<String>,
    pub team_id: Option<String>,
    pub response_cost: Option<Decimal>,
    pub span: Option<ParentOtelSpan>,
    pub tags: Option<Vec<Json>>,
}

#[must_use]
pub fn cache_read_keys(
    user_id: Option<&str>,
    end_user_id: Option<&str>,
    team_id: Option<&str>,
    tags: Option<&[Json]>,
    response_cost: Option<Decimal>,
) -> Vec<String> {
    if response_cost.is_none() {
        return Vec::new();
    }
    let mut keys = Vec::new();
    if let Some(user) = user_id {
        keys.push(user.to_owned());
        keys.push(GLOBAL_PROXY_SPEND_CACHE_KEY.to_owned());
    }
    if let Some(end_user) = end_user_id {
        keys.push(CacheKey::EndUserId(end_user).render());
    }
    if let Some(team) = team_id {
        keys.push(format!("team_id:{team}"));
    }
    for tag in tags.into_iter().flatten() {
        if let Json::Text(name) = tag
            && !name.is_empty()
        {
            keys.push(CacheKey::Tag(name.as_str()).render());
        }
    }
    keys
}

pub fn arm_cache_read(
    keys: &[String],
    cache: &dyn SpendCache,
) -> Result<(), SpendCacheUpdateError> {
    if keys.is_empty() {
        return Ok(());
    }
    let outcome = ACTIVE_REQUEST_BATCHES.try_with(|batches| {
        let Some(armed) = cache.declare(keys, batches)? else {
            return Ok(());
        };
        let _previous =
            lock(&batches.prefetched).insert(UPDATE_CACHE_READ_SLOT.to_owned(), Box::new(armed));
        Ok(())
    });
    match outcome {
        Ok(result) => result,
        Err(_no_request_batch) => Ok(()),
    }
}

// LiteLLM threads the parent OTel span into the read; B733 and B4294 carry no span, so it is accepted unused here.
#[must_use]
pub async fn read_cache_values(
    keys: &[String],
    _span: Option<&ParentOtelSpan>,
    cache: &dyn SpendCache,
) -> BTreeMap<String, Json> {
    if keys.is_empty() {
        return BTreeMap::new();
    }
    let read = match take_armed_read(keys) {
        Some(armed) => cache.resolve(armed).await,
        None => cache.read(keys, false).await,
    };
    match read {
        Ok(values) => keys
            .iter()
            .zip(values)
            .filter_map(|(key, cached)| cached.map(|value| (key.clone(), value)))
            .collect(),
        Err(error) => {
            warn!(
                "Spend tracking - failed to read cached spend objects. Budget enforcement may use stale spend values. keys={keys:?} - {error}"
            );
            BTreeMap::new()
        }
    }
}

fn take_armed_read(keys: &[String]) -> Option<ArmedSpendRead> {
    let slot = ACTIVE_REQUEST_BATCHES
        .try_with(|batches| lock(&batches.prefetched).remove(UPDATE_CACHE_READ_SLOT))
        .ok()
        .flatten()?;
    let armed = *slot.downcast::<ArmedSpendRead>().ok()?;
    (armed.keys.as_slice() == keys).then_some(armed)
}

pub async fn update_spend_cache(
    input: &UpdateInput,
    deps: &UpdateDeps,
) -> Result<(), SpendCacheUpdateError> {
    let keys = cache_read_keys(
        input.user_id.as_deref(),
        input.end_user_id.as_deref(),
        input.team_id.as_deref(),
        input.tags.as_deref(),
        input.response_cost,
    );
    let cached = read_cache_values(&keys, input.span.as_ref(), deps.cache.as_ref()).await;
    if let (Some(token), Some(response_cost)) = (input.token.as_deref(), input.response_cost) {
        update_key_spend(token, response_cost, deps).await?;
    }
    let mut updates = Vec::new();
    update_objects(input, &cached, &mut updates);
    spawn_writes(updates, deps);
    Ok(())
}

fn spawn_writes(updates: Vec<(String, Json)>, deps: &UpdateDeps) {
    let ttl = deps.cache.management_ttl();
    let (objects, global_spend): (Vec<_>, Vec<_>) = updates
        .into_iter()
        .partition(|(key, _value)| key != GLOBAL_PROXY_SPEND_CACHE_KEY);
    for (entries, local_only) in [(objects, true), (global_spend, false)] {
        if entries.is_empty() {
            continue;
        }
        let cache = Arc::clone(&deps.cache);
        deps.tasks.spawn(async move {
            cache.write(&entries, ttl, local_only).await;
        });
    }
}

pub(super) fn added_spend(
    existing: Decimal,
    response_cost: Decimal,
) -> Result<Decimal, SpendCacheUpdateError> {
    existing
        .checked_add(response_cost)
        .ok_or(SpendCacheUpdateError::SpendOverflow)
}
