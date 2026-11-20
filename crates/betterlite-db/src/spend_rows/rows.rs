use std::error::Error;
use std::future::Future;

use rust_decimal::Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpendRow {
    pub spend: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendRowLookup<'key> {
    Key(&'key str),
    TeamMember {
        user_id: &'key str,
        team_id: &'key str,
    },
    Team(&'key str),
    User(&'key str),
    Organization(&'key str),
    Project(&'key str),
    EndUser(&'key str),
}

// A failed row read travels through the bounded lookup as an answer, so the caller can log it and fall back instead of losing which failure happened
pub enum LookupAnswer<Row, Failure> {
    Row(Option<Row>),
    Failed(Failure),
}

pub trait SpendCounterRows: Send + Sync + 'static {
    type Error: Error + Send + Sync + 'static;

    fn spend_row(
        &self,
        lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<SpendRow>, Self::Error>> + Send;
}
