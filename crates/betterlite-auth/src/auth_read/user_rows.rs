use std::convert::Infallible;

use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use betterlite_db::user_repository::RowDecodeError;
use betterlite_db::user_repository::tables::USER_TABLE;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::include::{Including, IncludingQuery, UserInclude};

const USER_ONE_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_id\" = $1";

const USER_ONE_MEMBERS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_id\" = $1";

const USER_LIST_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry LIMIT $1";

const USER_LIST_MEMBERS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry LIMIT $1";

const USER_EMAIL_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_email\" = $1 LIMIT $2";

const USER_EMAIL_MEMBERS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_email\" = $1 LIMIT $2";

const USER_EMAIL_CASE_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry WHERE LOWER(entry.\"user_email\") = LOWER($1) LIMIT $2";

const USER_EMAIL_CASE_MEMBERS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry WHERE LOWER(entry.\"user_email\") = LOWER($1) LIMIT $2";

const USER_INSERT_SQL: &str = "INSERT INTO \"LiteLLM_UserTable\" AS entry (\"user_id\", \"user_alias\", \"team_id\", \"sso_user_id\", \"organization_id\", \"object_permission_id\", \"user_role\", \"user_email\", \"teams\", \"models\", \"max_parallel_requests\", \"tpm_limit\", \"rpm_limit\", \"budget_duration\", \"allowed_cache_controls\", \"policies\") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) RETURNING row_to_json(entry)::text";

const USER_INSERT_MEMBERS_SQL: &str = "WITH inserted AS (INSERT INTO \"LiteLLM_UserTable\" (\"user_id\", \"user_alias\", \"team_id\", \"sso_user_id\", \"organization_id\", \"object_permission_id\", \"user_role\", \"user_email\", \"teams\", \"models\", \"max_parallel_requests\", \"tpm_limit\", \"rpm_limit\", \"budget_duration\", \"allowed_cache_controls\", \"policies\") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16) RETURNING *) SELECT (to_jsonb(inserted) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = inserted.\"user_id\"), '[]'::jsonb)))::text FROM inserted";

const USER_UPDATE_SQL: &str = "UPDATE \"LiteLLM_UserTable\" AS entry SET \"user_id\" = $2, \"user_alias\" = $3, \"team_id\" = $4, \"sso_user_id\" = $5, \"organization_id\" = $6, \"object_permission_id\" = $7, \"user_role\" = $8, \"user_email\" = $9, \"teams\" = $10, \"models\" = $11, \"max_parallel_requests\" = $12, \"tpm_limit\" = $13, \"rpm_limit\" = $14, \"budget_duration\" = $15, \"allowed_cache_controls\" = $16, \"policies\" = $17 WHERE entry.\"user_id\" = $1 RETURNING row_to_json(entry)::text";

const USER_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_id\" = $1 RETURNING row_to_json(entry)::text";

const USER_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_UserTable\"";

const USER_COUNT_EMAIL_SQL: &str =
    "SELECT count(*) FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_email\" = $1";

const USER_COUNT_EMAIL_CASE_SQL: &str = "SELECT count(*) FROM \"LiteLLM_UserTable\" AS entry WHERE LOWER(entry.\"user_email\") = LOWER($1)";

// Prisma's user where clause: the email to match and whether the compare ignores case; no email matches every row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserEmailFilter {
    pub address: Option<String>,
    pub insensitive: bool,
}

// The user columns a native create supplies; every field binds and the database fills the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserCreate {
    pub user_id: String,
    pub user_alias: Option<String>,
    pub team_id: Option<String>,
    pub sso_user_id: Option<String>,
    pub organization_id: Option<String>,
    pub object_permission_id: Option<String>,
    pub user_role: Option<String>,
    pub user_email: Option<String>,
    pub teams: Vec<String>,
    pub models: Vec<String>,
    pub max_parallel_requests: Option<i32>,
    pub tpm_limit: Option<i64>,
    pub rpm_limit: Option<i64>,
    pub budget_duration: Option<String>,
    pub allowed_cache_controls: Vec<String>,
    pub policies: Vec<String>,
}

impl UserCreate {
    const fn parameters(&self) -> [&(dyn ToSql + Sync); 16] {
        [
            &self.user_id,
            &self.user_alias,
            &self.team_id,
            &self.sso_user_id,
            &self.organization_id,
            &self.object_permission_id,
            &self.user_role,
            &self.user_email,
            &self.teams,
            &self.models,
            &self.max_parallel_requests,
            &self.tpm_limit,
            &self.rpm_limit,
            &self.budget_duration,
            &self.allowed_cache_controls,
            &self.policies,
        ]
    }
}

// The native user table: a typed where clause for find_first and typed columns for create.
#[derive(Clone, Copy)]
pub struct UserRows;

impl RowCodec for UserRows {
    type Model = LiteLLMUserTable;
    type Error = RowDecodeError;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        USER_TABLE.decode(row)
    }
}

impl Table for UserRows {
    type Column = Infallible;
    type Filter = UserEmailFilter;
    type Order = Infallible;
    type Data = UserCreate;

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, Self::Data>,
        key: Key<'query, Self::Column>,
    ) -> Query<'query> {
        self.including(IncludingQuery::Keyed { action, key }, None)
    }

    fn list<'query>(
        &'query self,
        options: FindMany<'query, Self::Filter, Self::Order>,
    ) -> Query<'query> {
        self.including(
            IncludingQuery::Listed {
                filter: options.filter.and_then(|filters| filters.first()),
                limit: &NO_LIMIT,
            },
            None,
        )
    }

    fn insert<'query>(&'query self, data: &'query Self::Data) -> Query<'query> {
        self.including(IncludingQuery::Inserted { data }, None)
    }

    fn count<'query>(&'query self, filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        let predicate = filter.and_then(|filters| filters.first());
        let Some(address) = predicate.and_then(|selected| selected.address.as_ref()) else {
            return Query {
                sql: USER_COUNT_SQL,
                parameters: Vec::new(),
            };
        };
        Query {
            sql: if predicate.is_some_and(|selected| selected.insensitive) {
                USER_COUNT_EMAIL_CASE_SQL
            } else {
                USER_COUNT_EMAIL_SQL
            },
            parameters: vec![address],
        }
    }

    fn membership(&self, column: KeyColumn<Self::Column>) -> &str {
        USER_TABLE.membership(column)
    }
}

impl Including for UserRows {
    type Include = UserInclude;

    fn including<'query>(
        &'query self,
        query: IncludingQuery<'query, Self>,
        include: Option<&'query Self::Include>,
    ) -> Query<'query> {
        let memberships = include.is_some_and(|selected| selected.organization_memberships);
        match query {
            IncludingQuery::Keyed { action, key } => keyed_query(&action, &key, memberships),
            IncludingQuery::Listed { filter, limit } => list_query(filter, limit, memberships),
            IncludingQuery::Inserted { data } => insert_query(data, memberships),
        }
    }
}

// A bare table read has no borrowed limit, so the selecting table applies the request's take.
const NO_LIMIT: Option<i64> = None;

fn keyed_query<'query>(
    action: &KeyAction<'query, UserCreate>,
    key: &Key<'query, Infallible>,
    memberships: bool,
) -> Query<'query> {
    match (action, key.column) {
        (KeyAction::Find, KeyColumn::Id) => Query {
            sql: if memberships {
                USER_ONE_MEMBERS_SQL
            } else {
                USER_ONE_SQL
            },
            parameters: vec![key.value],
        },
        (KeyAction::Delete, KeyColumn::Id) => Query {
            sql: USER_DELETE_SQL,
            parameters: vec![key.value],
        },
        (KeyAction::Update(data), KeyColumn::Id) => {
            let mut parameters: Vec<&'query (dyn ToSql + Sync)> = vec![key.value];
            parameters.extend(data.parameters());
            Query {
                sql: USER_UPDATE_SQL,
                parameters,
            }
        }
        (
            KeyAction::Find | KeyAction::Delete | KeyAction::Update(_),
            KeyColumn::Alternate(never),
        ) => match never {},
    }
}

#[must_use]
pub fn list_query<'query>(
    filter: Option<&'query UserEmailFilter>,
    limit: &'query Option<i64>,
    memberships: bool,
) -> Query<'query> {
    let Some(address) = filter.and_then(|selected| selected.address.as_ref()) else {
        return Query {
            sql: if memberships {
                USER_LIST_MEMBERS_SQL
            } else {
                USER_LIST_SQL
            },
            parameters: vec![limit],
        };
    };
    Query {
        sql: match (
            filter.is_some_and(|selected| selected.insensitive),
            memberships,
        ) {
            (true, true) => USER_EMAIL_CASE_MEMBERS_SQL,
            (true, false) => USER_EMAIL_CASE_SQL,
            (false, true) => USER_EMAIL_MEMBERS_SQL,
            (false, false) => USER_EMAIL_SQL,
        },
        parameters: vec![address, limit],
    }
}

fn insert_query<'query>(data: &'query UserCreate, memberships: bool) -> Query<'query> {
    Query {
        sql: if memberships {
            USER_INSERT_MEMBERS_SQL
        } else {
            USER_INSERT_SQL
        },
        parameters: data.parameters().to_vec(),
    }
}
