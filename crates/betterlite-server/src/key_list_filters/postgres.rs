use std::convert::Infallible;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::database_refresh::handle::{PoolFuture, PoolTransport};
use betterlite_db::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use betterlite_db::database_replica::pinned::WriterPinnedClient;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::key_hash::KeyHash;
use betterlite_db::key_repository::VerificationTokenRepository;
use betterlite_db::key_repository::table::TOKEN_TABLE_ATTRIBUTE;
use betterlite_db::postgres_store::session::PostgresStore;
use deadpool_postgres::Pool;
use secrecy::SecretString;

use super::error::{KeyListFiltersError, KeyListReadError};
use super::ports::KeyListFiltersStore;
use crate::spend::tag_reports::store::{PostgresTagReportStore, TagReportStore as _};

pub struct PostgresKeyListFiltersStore {
    pub store: PostgresStore,
}

#[async_trait]
impl KeyListFiltersStore for PostgresKeyListFiltersStore {
    async fn user_row(
        &self,
        user_id: &str,
    ) -> Result<Option<LiteLLMUserTable>, KeyListFiltersError> {
        let reports = PostgresTagReportStore {
            store: self.store.clone(),
        };
        reports
            .user(user_id)
            .await
            .map_err(|error| KeyListFiltersError::Read(KeyListReadError::User(error)))
    }

    async fn key_by_hash(
        &self,
        key_hash: &KeyHash,
    ) -> Result<Option<LiteLLMVerificationToken>, KeyListFiltersError> {
        let client = PostgresTokenClient { store: &self.store };
        let repository = VerificationTokenRepository {
            database: Some(WriterPinnedClient::Writer(&client)),
        };
        let rows = repository
            .find_many_in(vec![key_hash.clone()])
            .await
            .map_err(|error| KeyListFiltersError::Read(KeyListReadError::Key(error)))?;
        Ok(rows.into_iter().next())
    }
}

// B722's repository only reads its table attribute, so the lifecycle steps never run
pub(crate) struct PostgresTokenClient<'store> {
    pub(crate) store: &'store PostgresStore,
}

impl DatabaseLifecycle for PostgresTokenClient<'_> {
    type Error = Infallible;

    #[cfg_attr(test, mutants::skip)]
    fn is_connected(&self) -> bool {
        self.store.pool.is_some()
    }

    #[cfg_attr(test, mutants::skip)]
    fn connect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn disconnect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn recreate<'database>(
        &'database self,
        _url: &'database SecretString,
        _transport: Option<&'database PoolTransport>,
        _expected_generation: Option<u64>,
    ) -> PoolFuture<'database, bool, Infallible> {
        Box::pin(async { Ok(false) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn start_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }

    #[cfg_attr(test, mutants::skip)]
    fn stop_refresh(&self) -> PoolFuture<'_, (), Infallible> {
        Box::pin(async { Ok(()) })
    }
}

impl DatabaseClient for PostgresTokenClient<'_> {
    type Attribute = Pool;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<Pool>>, Infallible> {
        if name == TOKEN_TABLE_ATTRIBUTE {
            return Ok(self.store.pool.clone().map(Attribute::Model));
        }
        Ok(None)
    }

    #[cfg_attr(test, mutants::skip)]
    fn token_refresh_enabled(&self) -> bool {
        false
    }

    #[cfg_attr(test, mutants::skip)]
    fn token_label(&self) -> &'static str {
        "database token"
    }

    #[cfg_attr(test, mutants::skip)]
    fn minted_url(&self) -> Result<Option<SecretString>, Infallible> {
        Ok(None)
    }
}
