use std::future::Future;
use std::sync::Arc;

use betterlite_auth::auth_key_lookup::KeyLookups;
use betterlite_auth::auth_key_lookup::key_object::{KeyRequest, get_key_object};
use betterlite_auth::auth_user_lookup::lookup::{UserLookup, UserLookupRequest};
use betterlite_auth::identity_store::IdentityStore;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::session::PostgresStore;
use chrono::{DateTime, Utc};

use crate::cli_team_grants::lookup::fetch_cli_sso_team_details;
use crate::cli_team_grants::records::CliSsoTeamDetail;
use crate::mcp::outbound_credentials::WallClock;

use self::errors::{identity_error, key_read_error, user_read_error};
use super::{IdentityLookup, SubjectReadError, SubjectStore, UserRowSource};

mod errors;
mod tests;

// The production SubjectStore: the same reads admission makes, as B8621 wires them.
pub struct LiveSubjectStore {
    pub key_lookups: Arc<KeyLookups>,
    pub user_lookup: Arc<UserLookup>,
    pub identities: IdentityStore,
    pub database: Option<Arc<PostgresStore>>,
    pub clock: Arc<dyn WallClock>,
}

impl LiveSubjectStore {
    // These delegates need a live auth database, so the error mapping above carries the tested behavior.
    #[cfg_attr(test, mutants::skip)]
    async fn key_answer(&self, key_hash: &str) -> Result<UserAPIKeyAuth, SubjectReadError> {
        let request = KeyRequest {
            hashed_token: key_hash,
            parent_otel_span: None,
            check_cache_only: None,
            check_db_only: false,
        };
        get_key_object(&self.key_lookups, &request)
            .await
            .map_err(|error| key_read_error(&error))
    }

    #[cfg_attr(test, mutants::skip)]
    async fn user_answer(
        &self,
        user_id: &str,
        source: UserRowSource,
    ) -> Result<Option<LiteLLMUserTable>, SubjectReadError> {
        let request = UserLookupRequest {
            user_id: Some(user_id),
            sso_user_id: None,
            user_email: None,
            user_id_upsert: false,
            check_db_only: Some(source == UserRowSource::Database),
        };
        self.user_lookup
            .get_user_object(request)
            .await
            .map_err(|error| user_read_error(&error))
    }

    #[cfg_attr(test, mutants::skip)]
    async fn identity_answer(&self, key_hash: &str) -> IdentityLookup {
        match self.identities.resolve(key_hash).await {
            Ok(_) => IdentityLookup::Found,
            Err(error) => identity_error(&error),
        }
    }
}

impl SubjectStore for LiveSubjectStore {
    #[cfg_attr(test, mutants::skip)]
    fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    #[cfg_attr(test, mutants::skip)]
    fn has_database(&self) -> bool {
        self.database.is_some()
    }

    fn key_by_hash(
        &self,
        key_hash: &str,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, SubjectReadError>> + Send {
        self.key_answer(key_hash)
    }

    fn user_by_id(
        &self,
        user_id: &str,
        source: UserRowSource,
    ) -> impl Future<Output = Result<Option<LiteLLMUserTable>, SubjectReadError>> + Send {
        self.user_answer(user_id, source)
    }

    fn identity_by_hash(&self, key_hash: &str) -> impl Future<Output = IdentityLookup> + Send {
        self.identity_answer(key_hash)
    }

    #[cfg_attr(test, mutants::skip)]
    async fn team_details(&self, teams: &[String]) -> Option<Vec<CliSsoTeamDetail>> {
        let Some(store) = self.database.as_ref() else {
            return None;
        };
        fetch_cli_sso_team_details(store.as_ref(), teams).await
    }
}
