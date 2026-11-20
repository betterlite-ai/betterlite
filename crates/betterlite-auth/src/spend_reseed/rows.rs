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

// the bounded lookup future must be 'static, so the decoded lookup owns the ids it reads
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum OwnedLookup {
    Key(String),
    TeamMember { user_id: String, team_id: String },
    Team(String),
    User(String),
    Organization(String),
    Project(String),
    EndUser(String),
}

impl OwnedLookup {
    #[must_use]
    pub(super) fn borrowed(&self) -> SpendRowLookup<'_> {
        match self {
            Self::Key(id) => SpendRowLookup::Key(id),
            Self::TeamMember { user_id, team_id } => {
                SpendRowLookup::TeamMember { user_id, team_id }
            }
            Self::Team(id) => SpendRowLookup::Team(id),
            Self::User(id) => SpendRowLookup::User(id),
            Self::Organization(id) => SpendRowLookup::Organization(id),
            Self::Project(id) => SpendRowLookup::Project(id),
            Self::EndUser(id) => SpendRowLookup::EndUser(id),
        }
    }
}

impl From<SpendRowLookup<'_>> for OwnedLookup {
    fn from(lookup: SpendRowLookup<'_>) -> Self {
        match lookup {
            SpendRowLookup::Key(id) => Self::Key(id.to_owned()),
            SpendRowLookup::TeamMember { user_id, team_id } => Self::TeamMember {
                user_id: user_id.to_owned(),
                team_id: team_id.to_owned(),
            },
            SpendRowLookup::Team(id) => Self::Team(id.to_owned()),
            SpendRowLookup::User(id) => Self::User(id.to_owned()),
            SpendRowLookup::Organization(id) => Self::Organization(id.to_owned()),
            SpendRowLookup::Project(id) => Self::Project(id.to_owned()),
            SpendRowLookup::EndUser(id) => Self::EndUser(id.to_owned()),
        }
    }
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
            CounterEntityType::TeamMember
            | CounterEntityType::User
            | CounterEntityType::EndUser
            | CounterEntityType::Tag
            | CounterEntityType::ModelAccessGroup
            | CounterEntityType::Organization
            | CounterEntityType::Project => None,
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
