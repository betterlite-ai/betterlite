use super::rows::{KeySpend, UserKeyDailySpend, UserModelSpend};
use super::store::{TotalsStore, TotalsStoreError};

// LiteLLM's key helper declares limit: int = 10 and the keys route never passes its own
pub(super) const INTERNAL_KEYS_LIMIT: i64 = 10;

#[derive(Debug, thiserror::Error)]
pub(super) enum InternalFailure {
    #[error("no database is connected")]
    NoDatabase,
    #[error("the internal user has no user id")]
    MissingUserId,
    #[error(transparent)]
    Store(#[from] TotalsStoreError),
}

pub(super) async fn global_spend_for_internal_user(
    store: Option<&dyn TotalsStore>,
    user_id: Option<&str>,
    api_key: Option<&str>,
) -> Result<Vec<UserKeyDailySpend>, InternalFailure> {
    let Some(store) = store else {
        return Err(InternalFailure::NoDatabase);
    };
    let Some(user_id) = user_id else {
        return Err(InternalFailure::MissingUserId);
    };
    Ok(store.user_daily_spend(user_id, api_key).await?)
}

pub(super) async fn global_spend_key_internal_user(
    store: Option<&dyn TotalsStore>,
    user_id: Option<&str>,
) -> Result<Vec<KeySpend>, InternalFailure> {
    let Some(store) = store else {
        return Err(InternalFailure::NoDatabase);
    };
    let Some(user_id) = user_id else {
        return Err(InternalFailure::MissingUserId);
    };
    Ok(store
        .user_keys_by_spend(user_id, INTERNAL_KEYS_LIMIT)
        .await?)
}

pub(super) async fn global_spend_models_internal(
    store: Option<&dyn TotalsStore>,
    user_id: Option<&str>,
    limit: i64,
) -> Result<Vec<UserModelSpend>, InternalFailure> {
    let Some(store) = store else {
        return Err(InternalFailure::NoDatabase);
    };
    let Some(user_id) = user_id else {
        return Err(InternalFailure::MissingUserId);
    };
    Ok(store.user_models_by_spend(user_id, limit).await?)
}
