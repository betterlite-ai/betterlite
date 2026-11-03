use crate::request_log_index_catalog::definitions::REQUEST_LOG_INDEXES;
use comparison::Comparison;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::time::timeout;
use tokio_postgres::GenericClient;

use thiserror::Error;
use tokio::time::error::Elapsed;

pub mod catalog;
pub mod comparison;
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectKind {
    Relation,
    Column,
    Constraint,
    Index,
    Sequence,
    Enum,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fact {
    pub kind: ObjectKind,
    pub relation: String,
    pub member: String,
    pub attribute: catalog::Attribute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaContract {
    pub facts: BTreeMap<Fact, Option<String>>,
}

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("catalog query failed")]
    Database(#[from] tokio_postgres::Error),
    #[error("catalog inspection timed out")]
    Timeout(#[from] Elapsed),
    #[error("namespace {0:?} does not exist")]
    Namespace(String),
    #[error("catalog returned an unknown attribute")]
    Attribute(#[from] strum::ParseError),
    #[error("catalog returned duplicate fact {0:?}")]
    Duplicate(Fact),
}
impl SchemaContract {
    pub async fn read<C>(
        session: &C,
        namespace: &str,
        duration: Duration,
    ) -> Result<Self, ContractError>
    where
        C: GenericClient + Sync,
    {
        let inspection = async {
            if session
                .query_opt(
                    "SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = $1",
                    &[&namespace],
                )
                .await?
                .is_none()
            {
                return Err(ContractError::Namespace(namespace.to_owned()));
            }
            let facts = catalog::collect(session, namespace).await?;
            Ok(Self { facts })
        };
        timeout(duration, inspection).await?
    }

    #[must_use]
    pub fn compare(&self, candidate: &Self) -> Comparison {
        let mut result = Comparison {
            differences: BTreeMap::new(),
            optional_expected: BTreeMap::new(),
            optional_actual: BTreeMap::new(),
        };
        for fact in self.facts.keys().chain(candidate.facts.keys()) {
            let expected = self.facts.get(fact);
            let actual = candidate.facts.get(fact);
            if fact.kind == ObjectKind::Index
                && REQUEST_LOG_INDEXES
                    .iter()
                    .any(|index| index.table == fact.relation && index.name == fact.member)
            {
                result
                    .optional_expected
                    .extend(expected.map(|value| (fact.clone(), value.clone())));
                result
                    .optional_actual
                    .extend(actual.map(|value| (fact.clone(), value.clone())));
                continue;
            }
            if let Some(change) = comparison::difference(expected, actual) {
                result.differences.insert(fact.clone(), change);
            }
        }
        result
    }
}
