use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::budget_counter::CounterEntityType;

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize)]
pub struct SpendRow {
    pub spend: Option<JsonData>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct WindowSpendRow {
    pub window_start: Timestamp,
    pub spend: Option<JsonData>,
}

// Prisma's group_by answers dict rows with a _sum mapping, or row objects with a _sum attribute
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendAggregateRow {
    Mapping(BTreeMap<String, JsonData>),
    Attribute { sum: Option<JsonData> },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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

// _WINDOW_SPEND_ENTITY_TYPES and _WINDOW_SPEND_LOG_FIELDS: only key and team window counters have a row
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowEntity {
    Key,
    Team,
}

impl WindowEntity {
    #[must_use]
    pub const fn from_counter_type(entity_type: &CounterEntityType) -> Option<Self> {
        match entity_type {
            CounterEntityType::Key => Some(Self::Key),
            CounterEntityType::Team => Some(Self::Team),
            _ => None,
        }
    }

    #[must_use]
    pub const fn table_label(self) -> &'static str {
        match self {
            Self::Key => "key",
            Self::Team => "team",
        }
    }
}

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

    fn window_row(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_duration: &str,
    ) -> impl Future<Output = Result<Option<WindowSpendRow>, Self::Error>> + Send;

    fn window_log_rows(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_start: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<SpendAggregateRow>, Self::Error>> + Send;
}
