use std::error::Error;
use std::sync::Arc;

use betterlite_config::sections::auth_user_lookup::DefaultUserParams;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::user_repository::projection::UserRecordError;
use betterlite_redact::secret::Redactor;
use tokio_util::task::TaskTracker;

use crate::auth_identity::errors::AuthError;
use crate::auth_read::budget_failure::log_budget_lookup_failure;
use crate::resolved_key::auth::AuthError as CredentialError;

use super::cache::{UserCache, UserCacheError};
use super::cadence::Cadence;
use super::create::create_user;
use super::error::{UserLookupError, user_read_failure};
use super::finish::{backfill, finish};
use super::fuzzy::fuzzy_user;
use super::ports::{BudgetResetCalculator, DefaultTeamEnrollment, PortError};
use super::store::{UserStore, UserStoreError};

// get_user_object's keyword arguments; check_db_only absent is the source's None
#[derive(Clone, Copy, Debug, Default)]
pub struct UserLookupRequest<'request> {
    pub user_id: Option<&'request str>,
    pub sso_user_id: Option<&'request str>,
    pub user_email: Option<&'request str>,
    pub user_id_upsert: bool,
    pub check_db_only: Option<bool>,
}

pub struct UserLookup {
    pub cache: Arc<dyn UserCache>,
    pub store: Option<Arc<dyn UserStore>>,
    pub cadence: Arc<Cadence>,
    pub db_cache_expiry: i64,
    pub budget_reset: Arc<dyn BudgetResetCalculator>,
    pub enrollment: Arc<dyn DefaultTeamEnrollment>,
    pub tasks: TaskTracker,
    pub defaults: Option<DefaultUserParams>,
    pub redactor: Arc<Redactor>,
}

// The source's try block: everything from the cadence check to the positive cadence write.
#[derive(Debug, thiserror::Error)]
pub(super) enum BlockFailure {
    #[error("the user was not found")]
    NotFound,
    #[error("the user row did not return")]
    NoRecord,
    #[error(transparent)]
    Store(Box<UserStoreError>),
    #[error(transparent)]
    Record(#[from] UserRecordError),
    #[error(transparent)]
    Membership(serde_json::Error),
    #[error(transparent)]
    Serialize(serde_json::Error),
    #[error(transparent)]
    Cache(UserCacheError),
    #[error(transparent)]
    Port(#[from] PortError),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the user create field {field} is not a user table column")]
    UnknownField { field: String },
}

impl BlockFailure {
    // the concrete failure, so the classifier can inspect its typed payload instead of a wrapper
    fn into_cause(self) -> Box<dyn Error + Send + Sync> {
        match self {
            Self::Store(error) => error,
            Self::Record(error) => Box::new(error),
            Self::Membership(error) | Self::Serialize(error) => Box::new(error),
            Self::Cache(error) => Box::new(error),
            Self::Port(error) => Box::new(error),
            Self::Credential(error) => Box::new(error),
            Self::NotFound => Box::new(Self::NotFound),
            Self::NoRecord => Box::new(Self::NoRecord),
            Self::UnknownField { field } => Box::new(Self::UnknownField { field }),
        }
    }
}

impl UserLookup {
    pub async fn get_user_object(
        &self,
        request: UserLookupRequest<'_>,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        let Some(user_id) = request.user_id else {
            return Ok(None);
        };
        let check_db_only = request.check_db_only.unwrap_or(false);
        if !check_db_only && let Some(cached) = self.cached_user(user_id).await? {
            let with_email = self.cached_backfill(cached, request.user_email).await?;
            return Ok(Some(with_email));
        }
        let Some(store) = self.store.as_deref() else {
            return Err(UserLookupError::NoDatabase);
        };
        let user = self
            .database_user(store, request, user_id, check_db_only)
            .await?;
        Ok(Some(user))
    }

    // async_get_cache(model_type=LiteLLM_UserTable): outside the try block, so a failure is not relabeled
    async fn cached_user(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, UserLookupError> {
        self.cache
            .read(user_id)
            .await
            .map_err(|error| UserLookupError::PreLookup(Box::new(error)))
    }

    // the cache hit still backfills a null email, with the same outside-the-catch failure rule
    async fn cached_backfill(
        &self,
        user: LiteLLMUserTable,
        email: Option<&str>,
    ) -> Result<LiteLLMUserTable, UserLookupError> {
        backfill(self, user, email)
            .await
            .map_err(|failure| UserLookupError::PreLookup(Box::new(failure)))
    }

    async fn database_user(
        &self,
        store: &dyn UserStore,
        request: UserLookupRequest<'_>,
        user_id: &str,
        check_db_only: bool,
    ) -> Result<LiteLLMUserTable, UserLookupError> {
        match self
            .read_user(store, &request, user_id, check_db_only)
            .await
        {
            Ok(user) => Ok(user),
            Err(BlockFailure::NotFound) => {
                Err(UserLookupError::NotFound(AuthError::UserNotFound {
                    user_id: user_id.to_owned(),
                }))
            }
            Err(failure) => {
                log_budget_lookup_failure(&self.redactor, "user", &failure);
                Err(user_read_failure(user_id, failure.into_cause()))
            }
        }
    }

    async fn read_user(
        &self,
        store: &dyn UserStore,
        request: &UserLookupRequest<'_>,
        user_id: &str,
        check_db_only: bool,
    ) -> Result<LiteLLMUserTable, BlockFailure> {
        let key = format!("user_id:{user_id}");
        let should_check =
            check_db_only || self.cadence.should_check_db(&key, self.db_cache_expiry);
        let primary = if should_check {
            let found = store
                .find_by_id(user_id, check_db_only)
                .await
                .map_err(|error| BlockFailure::Store(Box::new(error)))?;
            match found {
                Some(record) => Some(record),
                None => fuzzy_user(self, store, request.sso_user_id, request.user_email).await?,
            }
        } else {
            None
        };
        let record = match primary {
            Some(record) => record,
            None if !request.user_id_upsert => {
                if should_check {
                    self.cadence.update(&key, None);
                }
                return Err(BlockFailure::NotFound);
            }
            None => create_user(self, store, request, user_id).await?,
        };
        finish(self, record, request.user_email, user_id).await
    }
}
