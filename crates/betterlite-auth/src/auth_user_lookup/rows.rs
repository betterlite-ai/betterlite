use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use betterlite_db::user_repository::tables::USER_TABLE;
use tokio_postgres::Row;

use crate::auth_read::include::{Including, IncludingQuery, UserInclude};
use crate::auth_read::user_rows::{UserEmailFilter, list_query as email_query};

// B720 caps a stored row at 1 MiB; this task decodes the raw JSON itself, so the cap lives here too
const MAX_ROW_BYTES: usize = 0x0010_0000;

const USER_ONE_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_id\" = $1";

const USER_ONE_MEMBERS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_id\" = $1";

const USER_SSO_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"sso_user_id\" = $1 LIMIT $2";

const USER_SSO_MEMBERS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"sso_user_id\" = $1 LIMIT $2";

const USER_LIST_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_UserTable\" AS entry LIMIT $1";

const USER_LIST_MEMBERS_SQL: &str = "SELECT (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text FROM \"LiteLLM_UserTable\" AS entry LIMIT $1";

const USER_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_UserTable\"";

const USER_COUNT_SSO_SQL: &str =
    "SELECT count(*) FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"sso_user_id\" = $1";

const USER_COUNT_EMAIL_SQL: &str =
    "SELECT count(*) FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_email\" = $1";

const USER_COUNT_EMAIL_CASE_SQL: &str = "SELECT count(*) FROM \"LiteLLM_UserTable\" AS entry WHERE LOWER(entry.\"user_email\") = LOWER($1)";

const USER_UPDATE_SQL: &str = "UPDATE \"LiteLLM_UserTable\" AS entry SET \"user_id\" = entry.\"user_id\" WHERE entry.\"user_id\" = $1 RETURNING row_to_json(entry)::text";

const USER_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_UserTable\" AS entry WHERE entry.\"user_id\" = $1 RETURNING row_to_json(entry)::text";

const PLAIN_RETURNING: &str = " RETURNING row_to_json(entry)::text";

const MEMBERS_RETURNING: &str = " RETURNING (to_jsonb(entry) || jsonb_build_object('organization_memberships', COALESCE((SELECT jsonb_agg(member) FROM \"LiteLLM_OrganizationMembership\" AS member WHERE member.\"user_id\" = entry.\"user_id\"), '[]'::jsonb)))::text";

#[derive(Debug, thiserror::Error)]
pub enum UserRowError {
    #[error("the stored user row column is unreadable")]
    Column(#[source] DriverError),
    #[error("the stored user row is not readable JSON")]
    Json(#[source] serde_json::Error),
    #[error("the stored user row is {len} bytes, over the {MAX_ROW_BYTES} byte limit")]
    TooLarge { len: usize },
}

// Prisma's user where clause this lookup uses: an exact SSO id or a case-insensitive email
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserFilter {
    SsoId(String),
    Email(UserEmailFilter),
}

// The user columns a create writes; a merged create supplies only the ones its defaults name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserCreateData {
    pub(crate) fields: BTreeMap<String, JsonData>,
    pub(crate) json: String,
    pub(crate) members_sql: String,
    pub(crate) plain_sql: String,
}

impl UserCreateData {
    pub fn new(fields: BTreeMap<String, JsonData>) -> Result<Self, serde_json::Error> {
        let json = serde_json::to_string(&fields)?;
        Ok(Self {
            members_sql: insert_statement(&fields, true),
            plain_sql: insert_statement(&fields, false),
            fields,
            json,
        })
    }

    fn statement(&self, memberships: bool) -> &str {
        if memberships {
            return &self.members_sql;
        }
        &self.plain_sql
    }
}

// a dynamic column list over jsonb_populate_record, so a default only writes the columns it names
fn insert_statement(fields: &BTreeMap<String, JsonData>, memberships: bool) -> String {
    let returning = if memberships {
        MEMBERS_RETURNING
    } else {
        PLAIN_RETURNING
    };
    if fields.is_empty() {
        return format!("INSERT INTO \"LiteLLM_UserTable\" AS entry DEFAULT VALUES{returning}");
    }
    let columns: Vec<String> = fields
        .keys()
        .map(|name| format!("\"{}\"", name.replace('"', "\"\"")))
        .collect();
    let selected: Vec<String> = columns.iter().map(|column| format!("r.{column}")).collect();
    format!(
        "INSERT INTO \"LiteLLM_UserTable\" AS entry ({}) SELECT {} FROM jsonb_populate_record(NULL::\"LiteLLM_UserTable\", $1::text::jsonb) AS r{returning}",
        columns.join(", "),
        selected.join(", ")
    )
}

// The raw user table: rows stay JSON so a null membership entry survives until the lookup drops it.
#[derive(Clone, Copy)]
pub struct UserRows;

impl RowCodec for UserRows {
    type Model = JsonData;
    type Error = UserRowError;

    fn decode(&self, row: &Row) -> Result<Option<JsonData>, UserRowError> {
        let text: String = row
            .try_get(0)
            .map_err(|source| UserRowError::Column(DriverError { driver: source }))?;
        if text.len() > MAX_ROW_BYTES {
            return Err(UserRowError::TooLarge { len: text.len() });
        }
        serde_json::from_str(&text)
            .map(Some)
            .map_err(UserRowError::Json)
    }
}

impl Table for UserRows {
    type Column = Infallible;
    type Filter = UserFilter;
    type Order = Infallible;
    type Data = UserCreateData;

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
        filter_query(
            options.filter.and_then(|filters| filters.first()),
            &NO_LIMIT,
            false,
        )
    }

    fn insert<'query>(&'query self, data: &'query Self::Data) -> Query<'query> {
        self.including(IncludingQuery::Inserted { data }, None)
    }

    fn count<'query>(&self, filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        match filter {
            Some([UserFilter::SsoId(sso_user_id), ..]) => Query {
                sql: USER_COUNT_SSO_SQL,
                parameters: vec![sso_user_id],
            },
            Some([UserFilter::Email(email), ..]) => email.address.as_ref().map_or_else(
                || Query {
                    sql: USER_COUNT_SQL,
                    parameters: Vec::new(),
                },
                |address| Query {
                    sql: if email.insensitive {
                        USER_COUNT_EMAIL_CASE_SQL
                    } else {
                        USER_COUNT_EMAIL_SQL
                    },
                    parameters: vec![address],
                },
            ),
            Some([]) | None => Query {
                sql: USER_COUNT_SQL,
                parameters: Vec::new(),
            },
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
            IncludingQuery::Listed { filter, limit } => filter_query(filter, limit, memberships),
            IncludingQuery::Inserted { data } => Query {
                sql: data.statement(memberships),
                parameters: vec![&data.json],
            },
        }
    }
}

// A bare table read has no borrowed limit, so the selecting table applies the request's take.
const NO_LIMIT: Option<i64> = None;

fn keyed_query<'query>(
    action: &KeyAction<'query, UserCreateData>,
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
        (KeyAction::Update(_), KeyColumn::Id) => Query {
            sql: USER_UPDATE_SQL,
            parameters: vec![key.value],
        },
        (
            KeyAction::Find | KeyAction::Delete | KeyAction::Update(_),
            KeyColumn::Alternate(never),
        ) => match never {},
    }
}

// the SQL binds the limit as written, and a pub fn keeps clippy's ref_option quiet
#[must_use]
pub fn filter_query<'query>(
    filter: Option<&'query UserFilter>,
    limit: &'query Option<i64>,
    memberships: bool,
) -> Query<'query> {
    match filter {
        Some(UserFilter::SsoId(sso_user_id)) => Query {
            sql: if memberships {
                USER_SSO_MEMBERS_SQL
            } else {
                USER_SSO_SQL
            },
            parameters: vec![sso_user_id, limit],
        },
        Some(UserFilter::Email(email)) => email_query(Some(email), limit, memberships),
        None => Query {
            sql: if memberships {
                USER_LIST_MEMBERS_SQL
            } else {
                USER_LIST_SQL
            },
            parameters: vec![limit],
        },
    }
}
