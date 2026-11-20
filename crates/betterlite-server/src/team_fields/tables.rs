use std::sync::atomic::Ordering;

use betterlite_db::auxiliary_repository::bindings::{
    AccessGroupTable, DeletedTeamTable, OrganizationMembership, TeamMembership,
};
use betterlite_db::auxiliary_repository::repository::{
    PrismaClient, PrismaTableRepository, TableError,
};
use betterlite_db::database_replica::client::DatabaseClient;
use betterlite_db::database_replica::pinned::WriterPinnedClient;
use betterlite_db::key_repository::VerificationTokenRepository;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::{DbSession, PostgresStore};
use betterlite_db::resource_repository::budget::BudgetRepository;
use betterlite_db::resource_repository::model::ModelRepository;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_db::team_repository::decoder::TeamRecord;
use betterlite_db::team_repository::repository::{
    TeamDatabase, TeamQueryError, TeamRepository, TeamTable,
};
use betterlite_db::team_rows::member::Member;
use betterlite_db::user_repository::tables::ORGANIZATION_TABLE;
use betterlite_db::user_repository::{OrganizationRepository, StoredRepository, UserRepository};
use betterlite_redis::config_sync::publish::PublishChange;
use secrecy::SecretString;
use tokio_postgres::types::ToSql;

pub type TeamMembershipTable<'db> = PrismaTableRepository<'db, TeamMembership>;
pub type DeletedTeamRows<'db> = PrismaTableRepository<'db, DeletedTeamTable>;
pub type AccessGroupRows<'db> = PrismaTableRepository<'db, AccessGroupTable>;
pub type OrganizationMembershipRows<'db> = PrismaTableRepository<'db, OrganizationMembership>;

// LiteLLM's _team_db reads client.db and converts rows with _to_model; TeamRepository holds both paths.
pub fn team_table(client: Option<PrismaClient<'_>>) -> Result<TeamRepository<'_>, DbError> {
    let client = client.ok_or(DbError::NoDbConnected)?;
    Ok(TeamRepository {
        database: TeamDatabase {
            db: read_store(client),
            writer_db: client.writer,
        },
        use_writer: false,
    })
}

// B714 routes every Prisma read to the replica until its health flag says the replica is unavailable
fn read_store(client: PrismaClient<'_>) -> &PostgresStore {
    if client
        .availability
        .reader_unavailable
        .load(Ordering::SeqCst)
    {
        return client.writer;
    }
    client.reader
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's _raw_team_db exposes the same raw team table
pub fn raw_team_table<'database>(repository: &TeamRepository<'database>) -> TeamTable<'database> {
    repository.table()
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's _team_membership_db builds the membership table on the client
pub fn team_membership_table(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
) -> TeamMembershipTable<'_> {
    PrismaTableRepository::new(client, publish, TeamMembership)
}

pub const fn user_table<C>(database: Option<&C>) -> UserRepository<'_, C> {
    UserRepository {
        db: database,
        writer_db: None,
        use_writer: false,
    }
}

pub const fn organization_table<C>(database: Option<&C>) -> OrganizationRepository<'_, C> {
    StoredRepository {
        db: database,
        table: ORGANIZATION_TABLE,
    }
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's _org_membership_db builds the membership table on the client
pub fn organization_membership_table(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
) -> OrganizationMembershipRows<'_> {
    PrismaTableRepository::new(client, publish, OrganizationMembership)
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's _budget_db builds BudgetRepository on the supplied client
pub fn budget_table(client: Option<PrismaClient<'_>>) -> BudgetRepository<'_> {
    BudgetRepository::new(client)
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's _model_db builds ModelRepository on the supplied client
pub fn model_table(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
    encryption_key: Option<SecretString>,
    cipher: DbCipher,
) -> ModelRepository<'_> {
    ModelRepository::new(client, publish, encryption_key, cipher)
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's _deleted_team_db selects the deleted-team table
pub fn deleted_team_table(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
) -> DeletedTeamRows<'_> {
    PrismaTableRepository::new(client, publish, DeletedTeamTable)
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's _access_group_db selects the access-group table
pub fn access_group_table(
    client: Option<PrismaClient<'_>>,
    publish: PublishChange<TableError>,
) -> AccessGroupRows<'_> {
    PrismaTableRepository::new(client, publish, AccessGroupTable)
}

// ast-grep-ignore: no-one-line-functions - LiteLLM's _verification_token_db reads litellm_verificationtoken
pub fn tokens_table<C: DatabaseClient>(
    database: Option<WriterPinnedClient<'_, C>>,
) -> VerificationTokenRepository<'_, C> {
    VerificationTokenRepository { database }
}

const TEAM_ROW_SQL: &str =
    r#"SELECT row_to_json(team)::TEXT FROM "LiteLLM_TeamTable" team WHERE team_id = $1"#;

// LiteLLM's _team_tx_db keeps every table read on the transaction the endpoint already checked out.
pub struct TeamTransaction<'transaction, 'client> {
    pub session: &'transaction DbSession<'client>,
}

#[must_use]
// ast-grep-ignore: no-one-line-functions - LiteLLM's _team_tx_db wraps the caller's Prisma transaction
pub const fn team_transaction<'transaction, 'client>(
    session: &'transaction DbSession<'client>,
) -> TeamTransaction<'transaction, 'client> {
    TeamTransaction { session }
}

impl<'transaction, 'client> TeamTransaction<'transaction, 'client> {
    pub async fn team_row(&self, team_id: &str) -> Result<Option<TeamRecord>, TeamQueryError> {
        let found = self
            .session
            .query_opt(TEAM_ROW_SQL, &[&team_id])
            .await
            .map_err(backend)?;
        let Some(row) = found else {
            return Ok(None);
        };
        let text: String = row.try_get(0).map_err(backend)?;
        TeamRecord::parse(text.as_bytes()).map_err(TeamQueryError::from)
    }

    // ast-grep-ignore: no-one-line-functions - LiteLLM's _team_tx_db.members_with_roles reads the locked member list
    pub async fn members_with_roles(
        &self,
        team_id: &str,
    ) -> Result<Option<Vec<Member>>, TeamQueryError> {
        TeamRepository::get_members_with_roles_locked(self.session, team_id).await
    }

    // ast-grep-ignore: no-one-line-functions - _TeamDeleteTx.execute_raw runs the calling route's statement
    pub async fn execute_raw(
        &self,
        sql: &str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<u64, DbError> {
        self.session.execute(sql, parameters).await.map_err(backend)
    }

    // _MemberDeleteTx.litellm_usertable reads the user table on the same transaction, never a new connection.
    pub fn user_table(&self) -> UserRepository<'transaction, DbSession<'client>> {
        UserRepository {
            db: Some(self.session),
            writer_db: Some(self.session),
            use_writer: false,
        }
    }
}
