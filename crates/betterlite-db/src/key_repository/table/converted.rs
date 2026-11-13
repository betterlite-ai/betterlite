use super::{
    COUNT_TOKEN_SQL, DEFAULT_INSERT_SQL, DELETE_TOKEN_SQL, LIST_TOKEN_SQL, MEMBERSHIP_TOKEN_SQL,
    SELECT_TOKEN_SQL, TOUCH_TOKEN_SQL,
};
use crate::key_repository::TokenModel;
use crate::key_repository::decode::{TokenDecodeError, decode_token_record};
use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::rows::{FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table};
use std::convert::Infallible;
use tokio_postgres::Row;

pub(crate) struct ConvertedTokenTable;

impl RowCodec for ConvertedTokenTable {
    type Model = TokenModel;
    type Error = TokenDecodeError;

    fn decode(&self, row: &Row) -> Result<Option<Self::Model>, Self::Error> {
        let text: String = row.try_get(0).map_err(|driver| TokenDecodeError::Column {
            source: DriverError { driver },
        })?;
        decode_token_record(text.as_bytes()).map(Some)
    }
}

impl Table for ConvertedTokenTable {
    type Column = Infallible;
    type Filter = Infallible;
    type Order = Infallible;
    type Data = ();

    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, ()>,
        key: Key<'query, Infallible>,
    ) -> Query<'query> {
        match action {
            KeyAction::Find => Query {
                sql: SELECT_TOKEN_SQL,
                parameters: vec![key.value],
            },
            KeyAction::Delete => Query {
                sql: DELETE_TOKEN_SQL,
                parameters: vec![key.value],
            },
            KeyAction::Update(()) => Query {
                sql: TOUCH_TOKEN_SQL,
                parameters: vec![key.value],
            },
        }
    }

    fn list<'query>(
        &'query self,
        _options: FindMany<'query, Infallible, Infallible>,
    ) -> Query<'query> {
        Query {
            sql: LIST_TOKEN_SQL,
            parameters: Vec::new(),
        }
    }

    fn insert<'query>(&'query self, _data: &'query ()) -> Query<'query> {
        Query {
            sql: DEFAULT_INSERT_SQL,
            parameters: Vec::new(),
        }
    }

    fn count<'query>(&'query self, _filter: Option<&'query [Infallible]>) -> Query<'query> {
        Query {
            sql: COUNT_TOKEN_SQL,
            parameters: Vec::new(),
        }
    }

    fn membership(&self, column: KeyColumn<Infallible>) -> &str {
        match column {
            KeyColumn::Id => MEMBERSHIP_TOKEN_SQL,
            KeyColumn::Alternate(value) => match value {},
        }
    }
}
