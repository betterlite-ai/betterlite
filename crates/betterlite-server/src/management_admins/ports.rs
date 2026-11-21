use std::sync::Arc;

use async_trait::async_trait;
use betterlite_auth::auth_user_lookup::cache::UserCache;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_auth::auth_user_lookup::lookup::UserLookupRequest;
use betterlite_cache::backend::BackendError;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::decoder::{TeamRecord, TeamRecordError};

#[derive(Debug, thiserror::Error)]
pub enum UserReadError {
    #[error("the user lookup failed")]
    Lookup(#[source] UserLookupError),
    #[error("the empty user cache could not be built")]
    Cache(#[source] BackendError),
}

#[derive(Debug, thiserror::Error)]
pub enum TeamReadError {
    #[error("the team rows could not be read")]
    Database(#[source] DbError),
    #[error("a stored team row cannot be decoded")]
    Record(#[source] TeamRecordError),
}

// B736 get_user_object behind the supplied cache, or a fresh B2226 cache when the call supplies none
#[async_trait]
pub trait AdminUserReads: Send + Sync {
    fn fresh_cache(&self) -> Result<Arc<dyn UserCache>, UserReadError>;

    async fn user_object(
        &self,
        request: UserLookupRequest<'_>,
        cache: Arc<dyn UserCache>,
    ) -> Result<Option<LiteLLMUserTable>, UserReadError>;
}

// TeamRepository(prisma_client).table.find_many(where={"team_id": {"in": ids}})
#[async_trait]
pub trait TeamReads: Send + Sync {
    async fn teams_by_ids(&self, team_ids: &[String]) -> Result<Vec<TeamRecord>, TeamReadError>;
}
