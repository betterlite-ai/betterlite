use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

pub const PAGE_PARAM: &str = "page";
pub const PAGE_SIZE_PARAM: &str = "page_size";
pub const SORT_PARAM: &str = "sort";
pub const SEARCH_PARAM: &str = "q";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ComparisonOp {
    Eq,
    Gte,
    Lte,
    Gt,
    Lt,
    Contains,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FilterOp {
    Compare(ComparisonOp),
    In,
    IsNull,
}

impl FilterOp {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Compare(ComparisonOp::Eq) => "eq",
            Self::Compare(ComparisonOp::Gte) => "gte",
            Self::Compare(ComparisonOp::Lte) => "lte",
            Self::Compare(ComparisonOp::Gt) => "gt",
            Self::Compare(ComparisonOp::Lt) => "lt",
            Self::Compare(ComparisonOp::Contains) => "contains",
            Self::Compare(ComparisonOp::Not) => "not",
            Self::In => "in",
            Self::IsNull => "is_null",
        }
    }

    #[must_use]
    pub fn from_wire_name(name: &str) -> Option<Self> {
        match name {
            "eq" => Some(Self::Compare(ComparisonOp::Eq)),
            "gte" => Some(Self::Compare(ComparisonOp::Gte)),
            "lte" => Some(Self::Compare(ComparisonOp::Lte)),
            "gt" => Some(Self::Compare(ComparisonOp::Gt)),
            "lt" => Some(Self::Compare(ComparisonOp::Lt)),
            "contains" => Some(Self::Compare(ComparisonOp::Contains)),
            "not" => Some(Self::Compare(ComparisonOp::Not)),
            "in" => Some(Self::In),
            "is_null" => Some(Self::IsNull),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterType {
    Text,
    Int,
    Money,
    Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterValue {
    Text(String),
    Int(i64),
    Money(Decimal),
    Timestamp(DateTime<Utc>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compare {
    pub field: String,
    pub op: ComparisonOp,
    pub value: FilterValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Within {
    pub field: String,
    pub values: Vec<FilterValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsNull {
    pub field: String,
    pub negated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnyOf {
    pub clauses: Vec<Compare>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    Compare(Compare),
    Within(Within),
    IsNull(IsNull),
    AnyOf(AnyOf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterSpec {
    pub value_type: FilterType,
    pub ops: BTreeSet<FilterOp>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    pub field: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Where(Vec<Predicate>),
    Denied(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryPlan {
    pub where_: Vec<Predicate>,
    pub order: Vec<SortKey>,
    pub skip: u64,
    pub take: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ListSpecError {
    #[error(
        "{resource}: default_page_size must be between 1 and max_page_size ({max_page_size}), got {default_page_size}"
    )]
    DefaultPageSizeOutOfRange {
        resource: String,
        default_page_size: u64,
        max_page_size: u64,
    },
    #[error("{resource}: tiebreaker is required")]
    MissingTiebreaker { resource: String },
    #[error("{resource}: default_sort orders by non-sortable field(s): {fields}")]
    UndeclaredSortField { resource: String, fields: String },
    #[error(
        "{resource}: contains is only meaningful on text columns, but is declared on: {fields}"
    )]
    NonTextContainsFilter { resource: String, fields: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSpec {
    pub resource: String,
    pub sortable: BTreeSet<String>,
    pub searchable: BTreeSet<String>,
    pub filters: BTreeMap<String, FilterSpec>,
    pub default_sort: Vec<SortKey>,
    pub default_page_size: u64,
    pub max_page_size: u64,
    pub tiebreaker: String,
}

impl ListSpec {
    pub fn validate(self) -> Result<Self, ListSpecError> {
        if self.default_page_size < 1 || self.default_page_size > self.max_page_size {
            return Err(ListSpecError::DefaultPageSizeOutOfRange {
                resource: self.resource,
                default_page_size: self.default_page_size,
                max_page_size: self.max_page_size,
            });
        }
        if self.tiebreaker.is_empty() {
            return Err(ListSpecError::MissingTiebreaker {
                resource: self.resource,
            });
        }
        if let Some(fields) = self.undeclared_sort_fields() {
            return Err(ListSpecError::UndeclaredSortField {
                resource: self.resource,
                fields,
            });
        }
        if let Some(fields) = self.non_text_contains_filters() {
            return Err(ListSpecError::NonTextContainsFilter {
                resource: self.resource,
                fields,
            });
        }
        Ok(self)
    }

    fn undeclared_sort_fields(&self) -> Option<String> {
        let undeclared: BTreeSet<&str> = self
            .default_sort
            .iter()
            .map(|key| key.field.as_str())
            .filter(|field| !self.sortable.contains(*field))
            .collect();
        (!undeclared.is_empty()).then(|| undeclared.into_iter().collect::<Vec<_>>().join(", "))
    }

    fn non_text_contains_filters(&self) -> Option<String> {
        let non_text: Vec<&str> = self
            .filters
            .iter()
            .filter(|(_, spec)| {
                spec.value_type != FilterType::Text
                    && spec
                        .ops
                        .contains(&FilterOp::Compare(ComparisonOp::Contains))
            })
            .map(|(field, _)| field.as_str())
            .collect();
        (!non_text.is_empty()).then(|| non_text.join(", "))
    }
}

pub trait ListExecutor<Row> {
    fn count(&self, where_: &[Predicate]) -> impl Future<Output = u64> + Send;
    fn find_many(&self, plan: &QueryPlan) -> impl Future<Output = Vec<Row>> + Send;
}

pub trait FacetExecutor {
    fn distinct(
        &self,
        field: &str,
        where_: &[Predicate],
    ) -> impl Future<Output = Vec<String>> + Send;
}
