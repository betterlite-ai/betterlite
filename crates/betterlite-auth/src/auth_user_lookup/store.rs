use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::errors::{DbError as NativeDbError, backend};
use betterlite_db::postgres_store::rows::KeyColumn;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::user_repository::{RowDecodeError, UserRepository};
use deadpool_postgres::Client;
use tokio_util::task::TaskTracker;

use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::include::UserInclude;
use crate::auth_read::repository::{NativeKey, RepositoryTable};
use crate::auth_read::table::{
    AuthReadError, AuthReadTable, AuthTable, Create, FindOne, Presence, ReadContext,
};
use crate::auth_read::user_rows::UserEmailFilter;

use super::rows::{UserCreateData, UserFilter, UserRowError, UserRows};

pub type UserReadTable = AuthReadTable<RepositoryTable<Client, UserRows>>;
pub type UserStoreResult<Answer> = Result<Answer, UserStoreError>;

#[derive(Debug, thiserror::Error)]
pub enum UserStoreError {
    #[error("the user store session failed")]
    Session(#[source] NativeDbError),
    #[error("the user read failed")]
    Read(#[source] AuthReadError<UserRowError>),
    #[error("the user write failed")]
    Write(#[source] NativeDbError<UserRowError>),
    #[error("the null-email backfill failed")]
    Backfill(#[source] NativeDbError<RowDecodeError>),
    #[error("{0}")]
    Other(#[source] Box<dyn Error + Send + Sync>),
}

// _user_table(UserRepository(...)): find_unique/find_first for reads, create for a new user
#[async_trait]
pub trait UserStore: Send + Sync {
    async fn find_by_id(
        &self,
        user_id: &str,
        use_writer: bool,
    ) -> UserStoreResult<Option<JsonData>>;
    async fn find_by_sso_id(&self, sso_user_id: &str) -> UserStoreResult<Option<JsonData>>;
    async fn find_by_email(&self, email: &str) -> UserStoreResult<Option<JsonData>>;
    async fn create(&self, data: UserCreateData) -> UserStoreResult<JsonData>;
    async fn update_sso_user_id(&self, user_id: &str, sso_user_id: &str) -> UserStoreResult<()>;
    async fn backfill_null_email(&self, user_id: &str, email: &str) -> UserStoreResult<()>;
    async fn reread_for_backfill(&self, user_id: &str)
    -> UserStoreResult<Option<LiteLLMUserTable>>;
}

// B714 routing: a read falls back to the writer, and every write goes to the writer.
pub struct DatabaseUserStore {
    pub reader: PostgresStore,
    pub writer: PostgresStore,
    pub availability: Arc<Availability>,
    pub default_deadline_seconds: f64,
    pub tracker: Arc<DbLookupStallTracker<SystemClock>>,
    pub tasks: TaskTracker,
}

impl DatabaseUserStore {
    fn read_store(&self, use_writer: bool) -> &PostgresStore {
        if use_writer || self.availability.reader_unavailable.load(Ordering::SeqCst) {
            return &self.writer;
        }
        &self.reader
    }

    async fn open_on(&self, store: &PostgresStore) -> UserStoreResult<UserReadTable> {
        let session = store.session().await.map_err(UserStoreError::Session)?;
        let table = RepositoryTable {
            client: Some(Arc::new(session)),
            table: UserRows,
        };
        let context = ReadContext {
            default_deadline_seconds: self.default_deadline_seconds,
            tracker: Arc::clone(&self.tracker),
            tasks: self.tasks.clone(),
        };
        Ok(AuthReadAdapter::User.open(Arc::new(table), String::new(), context))
    }

    async fn find_one(
        &self,
        wanted: Wanted<'_>,
        use_writer: bool,
    ) -> UserStoreResult<Option<JsonData>> {
        let table = self.open_on(self.read_store(use_writer)).await?;
        let found = match wanted {
            Wanted::Id(user_id) => {
                table
                    .find_unique(FindOne {
                        filter: id_key(user_id),
                        include: members(),
                    })
                    .await
            }
            Wanted::Filter(filter) => {
                table
                    .find_first(FindOne {
                        filter,
                        include: members(),
                    })
                    .await
            }
        };
        found.map_err(UserStoreError::Read)
    }
}

// Prisma's find_unique over the user_id column and find_first over a where filter both answer one row
enum Wanted<'user_id> {
    Id(&'user_id str),
    Filter(UserFilter),
}

fn members() -> Presence<UserInclude> {
    Presence::Value(UserInclude {
        organization_memberships: true,
    })
}

fn id_key(user_id: &str) -> NativeKey<std::convert::Infallible> {
    NativeKey {
        column: KeyColumn::Id,
        value: Box::new(user_id.to_owned()),
    }
}

#[async_trait]
impl UserStore for DatabaseUserStore {
    async fn find_by_id(
        &self,
        user_id: &str,
        use_writer: bool,
    ) -> UserStoreResult<Option<JsonData>> {
        self.find_one(Wanted::Id(user_id), use_writer).await
    }

    async fn find_by_sso_id(&self, sso_user_id: &str) -> UserStoreResult<Option<JsonData>> {
        let filter = UserFilter::SsoId(sso_user_id.to_owned());
        self.find_one(Wanted::Filter(filter), false).await
    }

    async fn find_by_email(&self, email: &str) -> UserStoreResult<Option<JsonData>> {
        let filter = UserFilter::Email(UserEmailFilter {
            address: Some(email.to_owned()),
            insensitive: true,
        });
        self.find_one(Wanted::Filter(filter), false).await
    }

    async fn create(&self, data: UserCreateData) -> UserStoreResult<JsonData> {
        self.open_on(&self.writer)
            .await?
            .create(Create {
                data,
                include: members(),
            })
            .await
            .map_err(UserStoreError::Read)
    }

    async fn update_sso_user_id(&self, user_id: &str, sso_user_id: &str) -> UserStoreResult<()> {
        let session = self
            .writer
            .session()
            .await
            .map_err(UserStoreError::Session)?;
        let user_id = user_id.to_owned();
        let sso_user_id = sso_user_id.to_owned();
        let parameters: [&(dyn tokio_postgres::types::ToSql + Sync); 2] = [&user_id, &sso_user_id];
        session
            .execute(SSO_UPDATE_SQL, &parameters)
            .await
            .map(|_updated| ())
            .map_err(|source| UserStoreError::Write(backend(source)))
    }

    async fn backfill_null_email(&self, user_id: &str, email: &str) -> UserStoreResult<()> {
        let session = self
            .writer
            .session()
            .await
            .map_err(UserStoreError::Session)?;
        let repository = UserRepository {
            db: Some(&session),
            writer_db: None,
            use_writer: false,
        };
        repository
            .backfill_null_user_email(user_id, email)
            .await
            .map(|_updated| ())
            .map_err(UserStoreError::Backfill)
    }

    async fn reread_for_backfill(
        &self,
        user_id: &str,
    ) -> UserStoreResult<Option<LiteLLMUserTable>> {
        let session = self
            .read_store(false)
            .session()
            .await
            .map_err(UserStoreError::Session)?;
        let repository = UserRepository {
            db: Some(&session),
            writer_db: None,
            use_writer: false,
        };
        repository
            .find_by_id(user_id)
            .await
            .map_err(UserStoreError::Backfill)
    }
}

const SSO_UPDATE_SQL: &str =
    "UPDATE \"LiteLLM_UserTable\" AS entry SET \"sso_user_id\" = $2 WHERE entry.\"user_id\" = $1";
