use std::convert::Infallible;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_db::auxiliary_repository::bindings::TeamMembership;
use betterlite_db::auxiliary_repository::repository::{PrismaClient, PrismaTableRepository};
use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_failures::gate::{DbLookupStallTracker, LookupClock, SystemClock};
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_redis::config_sync::publish::redis_config_change_publisher;
use tokio_postgres::Row;
use tokio_util::task::TaskTracker;

use super::error::MembershipReadFailure;
use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::include::{Including, IncludingQuery};
use crate::auth_read::repository::RepositoryTable;
use crate::auth_read::table::{AuthReadError, AuthTable, FindOne, Presence, ReadContext};

const ONE_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_TeamMembership\" AS entry WHERE entry.\"user_id\" = $1 AND entry.\"team_id\" = $2 LIMIT $3";

const ONE_WITH_BUDGET_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('litellm_budget_table', (SELECT to_jsonb(budget) FROM \"LiteLLM_BudgetTable\" AS budget WHERE budget.\"budget_id\" = entry.\"budget_id\")))::text FROM \"LiteLLM_TeamMembership\" AS entry WHERE entry.\"user_id\" = $1 AND entry.\"team_id\" = $2 LIMIT $3";

const LIST_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_TeamMembership\" AS entry LIMIT $1";

const KEYED_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_TeamMembership\" AS entry WHERE entry.\"user_id\" = $1";

const INSERT_SQL: &str = "INSERT INTO \"LiteLLM_TeamMembership\" AS entry DEFAULT VALUES RETURNING row_to_json(entry)::text";

const COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_TeamMembership\"";

const NO_LIMIT: Option<i64> = None;

// The row's single json text column, or the reason it could not be read or parsed.
#[derive(Debug, thiserror::Error)]
pub enum MembershipRowError {
    #[error("stored row column could not be read")]
    Column(#[source] DriverError),
    #[error("stored row is not a readable record")]
    Record(#[source] serde_json::Error),
}

// Prisma's where for the composite primary key, and the one relation the lookup includes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MembershipFilter {
    pub user_id: String,
    pub team_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MembershipInclude {
    pub litellm_budget_table: bool,
}

#[derive(Clone, Copy)]
pub struct MembershipRows;

impl RowCodec for MembershipRows {
    type Model = LiteLLMTeamMembership;
    type Error = MembershipRowError;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        let text: String = row
            .try_get(0)
            .map_err(|driver| MembershipRowError::Column(DriverError { driver }))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(MembershipRowError::Record)
    }
}

impl Table for MembershipRows {
    type Column = Infallible;
    type Filter = MembershipFilter;
    type Order = Infallible;
    type Data = Infallible;

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, Self::Data>,
        key: Key<'query, Self::Column>,
    ) -> Query<'query> {
        match (action, key.column) {
            (KeyAction::Find | KeyAction::Delete | KeyAction::Update(_), KeyColumn::Id) => Query {
                sql: KEYED_SQL,
                parameters: vec![key.value],
            },
            (
                KeyAction::Find | KeyAction::Delete | KeyAction::Update(_),
                KeyColumn::Alternate(never),
            ) => match never {},
        }
    }

    fn list<'query>(
        &'query self,
        options: FindMany<'query, Self::Filter, Self::Order>,
    ) -> Query<'query> {
        listed(
            options.filter.and_then(|filters| filters.first()),
            &NO_LIMIT,
            false,
        )
    }

    fn insert<'query>(&'query self, _data: &'query Self::Data) -> Query<'query> {
        Query {
            sql: INSERT_SQL,
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&'query self, _filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        Query {
            sql: COUNT_SQL,
            parameters: Vec::new(),
        }
    }

    fn membership(&self, column: KeyColumn<Self::Column>) -> &str {
        match column {
            KeyColumn::Id => KEYED_SQL,
            KeyColumn::Alternate(never) => match never {},
        }
    }
}

impl Including for MembershipRows {
    type Include = MembershipInclude;

    fn including<'query>(
        &'query self,
        query: IncludingQuery<'query, Self>,
        include: Option<&'query Self::Include>,
    ) -> Query<'query> {
        let with_budget = include.is_some_and(|selected| selected.litellm_budget_table);
        match query {
            IncludingQuery::Keyed { action, key } => self.keyed(action, key),
            IncludingQuery::Listed { filter, limit } => listed(filter, limit, with_budget),
            IncludingQuery::Inserted { data } => self.insert(data),
        }
    }
}

// The composite primary key bounds the read to one row, and only an included budget adds the relation.
#[must_use]
pub fn listed<'query>(
    filter: Option<&'query MembershipFilter>,
    limit: &'query Option<i64>,
    with_budget: bool,
) -> Query<'query> {
    let Some(filter) = filter else {
        return Query {
            sql: LIST_SQL,
            parameters: vec![limit],
        };
    };
    let sql = if with_budget {
        ONE_WITH_BUDGET_SQL
    } else {
        ONE_SQL
    };
    Query {
        sql,
        parameters: vec![&filter.user_id, &filter.team_id, limit],
    }
}

// The membership read the lookup needs; the database adapter and the tests both answer it.
#[async_trait]
pub trait TeamMembershipRows: Send + Sync {
    async fn find_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipReadFailure>;
}

// auth_checks.py's find_unique with the budget included, through B2315's bounded table.
pub async fn read_membership<T, Clock>(
    table: Arc<T>,
    context: ReadContext<Clock>,
    user_id: &str,
    team_id: &str,
) -> Result<Option<LiteLLMTeamMembership>, MembershipReadFailure>
where
    T: AuthTable<
            Row = LiteLLMTeamMembership,
            Filter = MembershipFilter,
            Include = MembershipInclude,
            Failure = MembershipRowError,
        >,
    Clock: LookupClock + 'static,
{
    let bounded = AuthReadAdapter::Dictable.open(table, "team_membership".to_owned(), context);
    let request = FindOne {
        filter: MembershipFilter {
            user_id: user_id.to_owned(),
            team_id: team_id.to_owned(),
        },
        include: Presence::Value(MembershipInclude {
            litellm_budget_table: true,
        }),
    };
    bounded
        .find_first(request)
        .await
        .map_err(|source| MembershipReadFailure {
            source: Box::new(source),
        })
}

// B714's read routing answers from the reader unless it is unavailable, and this adapter never writes.
pub struct DatabaseTeamMembership {
    pub writer: Arc<PostgresStore>,
    pub reader: Arc<PostgresStore>,
    pub availability: Arc<Availability>,
    pub default_deadline_seconds: f64,
    pub tracker: Arc<DbLookupStallTracker<SystemClock>>,
    pub tasks: TaskTracker,
}

#[async_trait]
impl TeamMembershipRows for DatabaseTeamMembership {
    async fn find_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipReadFailure> {
        let client = PrismaClient {
            writer: self.writer.as_ref(),
            reader: self.reader.as_ref(),
            availability: self.availability.as_ref(),
        };
        let repository = PrismaTableRepository::new(
            Some(client),
            redis_config_change_publisher(None),
            TeamMembership,
        );
        let access = repository.access().map_err(read_fault)?;
        let session = access.read.session().await.map_err(read_fault)?;
        let table = RepositoryTable {
            client: Some(Arc::new(session)),
            table: MembershipRows,
        };
        read_membership(
            Arc::new(table),
            ReadContext {
                default_deadline_seconds: self.default_deadline_seconds,
                tracker: Arc::clone(&self.tracker),
                tasks: self.tasks.clone(),
            },
            user_id,
            team_id,
        )
        .await
    }
}

// The store reports DbError<Infallible>, a lookup-level database failure, before any row is decoded.
fn read_fault(source: DbError) -> MembershipReadFailure {
    MembershipReadFailure {
        source: Box::new(AuthReadError::Lookup(DbLookupError::Database(source))),
    }
}
