use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::postgres_store::session::PostgresStore;

use crate::auth_end_users::cache::EndUserCache;
use crate::auth_end_users::object::{EndUserObjectError, EndUserObjects, EndUserRequest};
use crate::auth_end_users::rows::{BudgetRows, EndUserRows};
use crate::auth_user_lookup::error::UserLookupError;
use crate::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use crate::end_user_registry::EndUserRegistry;

#[derive(Debug, thiserror::Error)]
pub enum ExistenceError {
    #[error("the end-user row lookup failed")]
    EndUser(#[source] EndUserObjectError),
    #[error("the user row lookup failed")]
    User(#[source] UserLookupError),
}

// whether LiteLLM_EndUserTable holds the id, over B739's get_end_user_object
#[async_trait]
pub trait EndUserExistence: Send + Sync {
    async fn end_user_exists(&self, end_user_id: &str) -> Result<bool, ExistenceError>;
}

// what the existence probe asks of B736's get_user_object before its own cache and cadence run
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserProbe<'probe> {
    pub user_id: &'probe str,
    pub user_email: Option<&'probe str>,
    pub user_id_upsert: bool,
    pub check_db_only: bool,
}

// whether LiteLLM_UserTable holds the id or the email, over B736's get_user_object
#[async_trait]
pub trait UserExistence: Send + Sync {
    async fn user_exists(&self, probe: &UserProbe<'_>) -> Result<bool, ExistenceError>;
}

impl<'probe> From<&'probe UserProbe<'probe>> for UserLookupRequest<'probe> {
    fn from(probe: &'probe UserProbe<'probe>) -> Self {
        Self {
            user_id: Some(probe.user_id),
            sso_user_id: None,
            user_email: probe.user_email,
            user_id_upsert: probe.user_id_upsert,
            check_db_only: Some(probe.check_db_only),
        }
    }
}

#[async_trait]
impl UserExistence for UserLookup {
    async fn user_exists(&self, probe: &UserProbe<'_>) -> Result<bool, ExistenceError> {
        self.get_user_object(UserLookupRequest::from(probe))
            .await
            .map(|found| found.is_some())
            .map_err(ExistenceError::User)
    }
}

// B739's get_end_user_object over whatever rows, budget rows and management cache the proxy wired
pub struct EndUserObjectExistence<
    Rows: EndUserRows + ?Sized,
    Budgets: BudgetRows + ?Sized,
    Cache: EndUserCache + ?Sized,
> {
    pub rows: Option<Arc<Rows>>,
    pub budgets: Option<Arc<Budgets>>,
    pub cache: Arc<Cache>,
    pub registry: Arc<EndUserRegistry>,
    pub store: PostgresStore,
    pub max_end_user_budget_id: Option<String>,
}

#[async_trait]
impl<Rows, Budgets, Cache> EndUserExistence for EndUserObjectExistence<Rows, Budgets, Cache>
where
    Rows: EndUserRows + ?Sized,
    Budgets: BudgetRows + ?Sized,
    Cache: EndUserCache + ?Sized,
{
    async fn end_user_exists(&self, end_user_id: &str) -> Result<bool, ExistenceError> {
        let Some(rows) = self.rows.as_deref() else {
            return Err(ExistenceError::EndUser(EndUserObjectError::NoDatabase));
        };
        let objects = EndUserObjects {
            rows: Some(rows),
            budgets: self.budgets.as_deref(),
            cache: self.cache.as_ref(),
            registry: self.registry.as_ref(),
            store: &self.store,
            max_end_user_budget_id: self.max_end_user_budget_id.as_deref(),
        };
        objects
            .get_end_user_object(&EndUserRequest {
                end_user_id: Some(end_user_id),
                key_end_user_budget_id: None,
                token_end_user_max_budget: None,
            })
            .await
            .map(|found| found.is_some())
            .map_err(ExistenceError::EndUser)
    }
}
