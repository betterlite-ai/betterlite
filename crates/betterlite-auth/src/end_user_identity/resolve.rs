use std::fmt;
use std::sync::Arc;

use betterlite_models::decisions::Json;

use super::cache::{VALIDATION_KEY_PREFIX, ValidationCache, Verdict};
use super::existence::{EndUserExistence, UserExistence, UserProbe};
use crate::auth_cache::AuthCacheError;

// the two existence probes an unknown id can match, end users first
pub struct DatabaseLookups {
    pub end_users: Arc<dyn EndUserExistence>,
    pub users: Arc<dyn UserExistence>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResolveRequest<'request> {
    pub raw_end_user_id: Option<&'request str>,
    pub key_end_user_budget_id: Option<&'request str>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("the end-user validation cache could not be read")]
    CacheRead(#[source] AuthCacheError),
    #[error("the end-user validation verdict could not be cached")]
    CacheWrite(#[source] AuthCacheError),
}

// LiteLLM's resolve_and_validate_end_user_id over the configured flag, the lookups and the raw cache
pub struct EndUserIdResolver {
    pub validate_end_user_id_in_db: bool,
    pub max_end_user_budget_id: Option<String>,
    pub lookups: Option<DatabaseLookups>,
    pub cache: Arc<dyn ValidationCache>,
}

// the wired ports stay out of the log: only which probes exist is readable
impl fmt::Debug for EndUserIdResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EndUserIdResolver")
            .field(
                "validate_end_user_id_in_db",
                &self.validate_end_user_id_in_db,
            )
            .field("max_end_user_budget_id", &self.max_end_user_budget_id)
            .field("lookups", &self.lookups.is_some())
            .finish_non_exhaustive()
    }
}

impl EndUserIdResolver {
    pub async fn resolve(
        &self,
        request: &ResolveRequest<'_>,
    ) -> Result<Option<String>, ResolveError> {
        let Some(raw_end_user_id) = request.raw_end_user_id else {
            return Ok(None);
        };
        if !self.validate_end_user_id_in_db {
            return Ok(Some(raw_end_user_id.to_owned()));
        }
        let Some(lookups) = &self.lookups else {
            return Ok(Some(raw_end_user_id.to_owned()));
        };
        // Python's bool(): an empty proxy default is unset, while any key value, even "", was supplied
        let has_default_budget = self
            .max_end_user_budget_id
            .as_deref()
            .is_some_and(|budget_id| !budget_id.is_empty())
            || request.key_end_user_budget_id.is_some();
        let cache_key = format!("{VALIDATION_KEY_PREFIX}{raw_end_user_id}");
        let cached = self
            .cache
            .read(&cache_key)
            .await
            .map_err(ResolveError::CacheRead)?;
        if let Some(verdict) = cached.as_ref().and_then(Verdict::of_cached) {
            return Ok(match verdict {
                Verdict::Valid => Some(raw_end_user_id.to_owned()),
                Verdict::Invalid => has_default_budget.then(|| raw_end_user_id.to_owned()),
            });
        }
        let is_valid = exists_in_db(lookups, raw_end_user_id).await;
        let verdict = if is_valid {
            Verdict::Valid
        } else {
            Verdict::Invalid
        };
        self.cache
            .write(
                &cache_key,
                &Json::Text(verdict.text().to_owned()),
                verdict.ttl(),
            )
            .await
            .map_err(ResolveError::CacheWrite)?;
        Ok((is_valid || has_default_budget).then(|| raw_end_user_id.to_owned()))
    }
}

// either lookup answering a row validates the id; a failure only logs and falls through
async fn exists_in_db(lookups: &DatabaseLookups, end_user_id: &str) -> bool {
    let end_user_found = match lookups.end_users.end_user_exists(end_user_id).await {
        Ok(found) => found,
        Err(error) => {
            tracing::debug!("end_user validation: get_end_user_object lookup failed: {error}");
            false
        }
    };
    if end_user_found {
        return true;
    }
    let probe = UserProbe {
        user_id: end_user_id,
        user_email: end_user_id.contains('@').then_some(end_user_id),
        user_id_upsert: false,
        check_db_only: false,
    };
    match lookups.users.user_exists(&probe).await {
        Ok(found) => found,
        Err(error) => {
            tracing::debug!("end_user validation: get_user_object lookup failed: {error}");
            false
        }
    }
}
