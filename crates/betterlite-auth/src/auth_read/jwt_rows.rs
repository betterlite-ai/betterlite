use std::convert::Infallible;

use betterlite_db::postgres_store::diagnostics::DriverError;
use betterlite_db::postgres_store::rows::{
    FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table,
};
use serde::Deserialize;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::include::{Including, IncludingQuery};

const JWT_ONE_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_JWTKeyMapping\" AS entry WHERE entry.\"id\" = $1";

const JWT_DELETE_SQL: &str = "DELETE FROM \"LiteLLM_JWTKeyMapping\" AS entry WHERE entry.\"id\" = $1 RETURNING row_to_json(entry)::text";

const JWT_LIST_SQL: &str =
    "SELECT row_to_json(entry)::text FROM \"LiteLLM_JWTKeyMapping\" AS entry LIMIT $1";

const JWT_FILTER_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_JWTKeyMapping\" AS entry WHERE entry.\"jwt_claim_name\" = $1 AND entry.\"jwt_claim_value\" = $2 AND entry.\"jwt_issuer\" = $3 AND entry.\"is_active\" = $4 LIMIT $5";

const JWT_INSERT_SQL: &str = "INSERT INTO \"LiteLLM_JWTKeyMapping\" AS entry (\"id\", \"jwt_issuer\", \"jwt_claim_name\", \"jwt_claim_value\", \"token\", \"description\", \"is_active\", \"created_by\", \"updated_by\") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING row_to_json(entry)::text";

const JWT_UPDATE_SQL: &str = "UPDATE \"LiteLLM_JWTKeyMapping\" AS entry SET \"id\" = $2, \"jwt_issuer\" = $3, \"jwt_claim_name\" = $4, \"jwt_claim_value\" = $5, \"token\" = $6, \"description\" = $7, \"is_active\" = $8, \"created_by\" = $9, \"updated_by\" = $10 WHERE entry.\"id\" = $1 RETURNING row_to_json(entry)::text";

const JWT_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_JWTKeyMapping\"";

const JWT_COUNT_FILTER_SQL: &str = "SELECT count(*) FROM \"LiteLLM_JWTKeyMapping\" AS entry WHERE entry.\"jwt_claim_name\" = $1 AND entry.\"jwt_claim_value\" = $2 AND entry.\"jwt_issuer\" = $3 AND entry.\"is_active\" = $4";

const JWT_MEMBERSHIP_SQL: &str = "SELECT row_to_json(entry)::text FROM \"LiteLLM_JWTKeyMapping\" AS entry WHERE entry.\"id\" = ANY($1::text[])";

// The mapping columns LiteLLM's row protocol exposes; other stored columns stay out of the view.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct JwtMappingRow {
    pub token: String,
    pub jwt_issuer: String,
    pub jwt_claim_name: String,
    pub jwt_claim_value: String,
}

#[derive(Debug, thiserror::Error)]
pub enum JwtRowError {
    #[error("stored jwt mapping row column could not be read")]
    Column(#[source] DriverError),
    #[error("stored jwt mapping row is not a readable record")]
    Record(#[source] serde_json::Error),
}

// LiteLLM's JWT-to-key where clause: the claim pair, its issuer scope and the active flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JwtMappingFilter {
    pub claim_name: String,
    pub claim_value: String,
    pub issuer: String,
    pub active: bool,
}

impl JwtMappingFilter {
    fn parameters(&self) -> Vec<&(dyn ToSql + Sync)> {
        vec![
            &self.claim_name,
            &self.claim_value,
            &self.issuer,
            &self.active,
        ]
    }
}

// The mapping columns a native create supplies; every field binds and the database fills the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JwtMappingCreate {
    pub id: String,
    pub jwt_issuer: String,
    pub jwt_claim_name: String,
    pub jwt_claim_value: String,
    pub token: String,
    pub description: Option<String>,
    pub is_active: bool,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
}

impl JwtMappingCreate {
    const fn parameters(&self) -> [&(dyn ToSql + Sync); 9] {
        [
            &self.id,
            &self.jwt_issuer,
            &self.jwt_claim_name,
            &self.jwt_claim_value,
            &self.token,
            &self.description,
            &self.is_active,
            &self.created_by,
            &self.updated_by,
        ]
    }
}

// The native JWT mapping table: a compound typed where clause and typed columns for create.
#[derive(Clone, Copy)]
pub struct JwtRows;

impl RowCodec for JwtRows {
    type Model = JwtMappingRow;
    type Error = JwtRowError;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        let text: String = row
            .try_get(0)
            .map_err(|driver| JwtRowError::Column(DriverError { driver }))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(JwtRowError::Record)
    }
}

impl Table for JwtRows {
    type Column = Infallible;
    type Filter = JwtMappingFilter;
    type Order = Infallible;
    type Data = JwtMappingCreate;

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, Self::Data>,
        key: Key<'query, Self::Column>,
    ) -> Query<'query> {
        match (action, key.column) {
            (KeyAction::Find, KeyColumn::Id) => Query {
                sql: JWT_ONE_SQL,
                parameters: vec![key.value],
            },
            (KeyAction::Delete, KeyColumn::Id) => Query {
                sql: JWT_DELETE_SQL,
                parameters: vec![key.value],
            },
            (KeyAction::Update(data), KeyColumn::Id) => {
                let mut parameters: Vec<&'query (dyn ToSql + Sync)> = vec![key.value];
                parameters.extend(data.parameters());
                Query {
                    sql: JWT_UPDATE_SQL,
                    parameters,
                }
            }
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
        jwt_list(
            options.filter.and_then(|filters| filters.first()),
            &NO_LIMIT,
        )
    }

    fn insert<'query>(&'query self, data: &'query Self::Data) -> Query<'query> {
        Query {
            sql: JWT_INSERT_SQL,
            parameters: data.parameters().to_vec(),
        }
    }

    fn count<'query>(&'query self, filter: Option<&'query [Self::Filter]>) -> Query<'query> {
        let Some(predicate) = filter.and_then(|filters| filters.first()) else {
            return Query {
                sql: JWT_COUNT_SQL,
                parameters: Vec::new(),
            };
        };
        Query {
            sql: JWT_COUNT_FILTER_SQL,
            parameters: predicate.parameters().to_vec(),
        }
    }

    fn membership(&self, _column: KeyColumn<Self::Column>) -> &str {
        JWT_MEMBERSHIP_SQL
    }
}

impl Including for JwtRows {
    type Include = ();

    fn including<'query>(
        &'query self,
        query: IncludingQuery<'query, Self>,
        _include: Option<&'query Self::Include>,
    ) -> Query<'query> {
        match query {
            IncludingQuery::Keyed { action, key } => self.keyed(action, key),
            IncludingQuery::Listed { filter, limit } => jwt_list(filter, limit),
            IncludingQuery::Inserted { data } => self.insert(data),
        }
    }
}

// A bare table read has no borrowed limit, so the selecting table applies the request's take.
const NO_LIMIT: Option<i64> = None;

#[must_use]
pub fn jwt_list<'query>(
    filter: Option<&'query JwtMappingFilter>,
    limit: &'query Option<i64>,
) -> Query<'query> {
    let Some(predicate) = filter else {
        return Query {
            sql: JWT_LIST_SQL,
            parameters: vec![limit],
        };
    };
    let mut parameters = predicate.parameters();
    parameters.push(limit);
    Query {
        sql: JWT_FILTER_SQL,
        parameters,
    }
}
