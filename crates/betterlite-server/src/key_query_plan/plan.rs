use chrono::{DateTime, Utc};

use betterlite_db::table_ops::access::KEY_TABLE;

use crate::key_query_plan::error::KeyQueryPlanError;
use crate::key_query_plan::filters::globals::build_key_filter_conditions;
use crate::key_query_plan::filters::{KeyFilters, WhereClause};
use crate::key_query_plan::sort::{DEFAULT_SORT, SortBy, validate_sort_column};

pub const DELETED_KEY_TABLE: &str = "LiteLLM_DeletedVerificationToken";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTable {
    Live,
    Deleted,
}

impl KeyTable {
    #[must_use]
    pub const fn sql_name(self) -> &'static str {
        match self {
            Self::Live => KEY_TABLE,
            Self::Deleted => DELETED_KEY_TABLE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyListQuery {
    pub now: DateTime<Utc>,
    pub page: i64,
    pub size: i64,
    pub filters: KeyFilters,
    pub sort_by: Option<String>,
    pub sort_order: String,
    pub expand: Vec<String>,
    pub return_full_object: bool,
}

impl KeyListQuery {
    #[must_use]
    pub fn new(now: DateTime<Utc>, page: i64, size: i64) -> Self {
        Self {
            now,
            page,
            size,
            filters: KeyFilters::default(),
            sort_by: None,
            sort_order: String::from("desc"),
            expand: Vec::new(),
            return_full_object: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyQueryPlan {
    pub table: KeyTable,
    pub where_clause: WhereClause,
    pub sort: Vec<SortBy>,
    pub skip: i64,
    pub take: i64,
}

pub fn build_plan(query: &KeyListQuery) -> Result<KeyQueryPlan, KeyQueryPlanError> {
    // Prisma divides by the page size for total_pages, and LIMIT cannot be negative
    if query.size <= 0 {
        return Err(KeyQueryPlanError::Pagination);
    }
    let where_clause = build_key_filter_conditions(&query.filters, query.now);
    let skip = query
        .page
        .checked_sub(1)
        .and_then(|page| page.checked_mul(query.size))
        .ok_or(KeyQueryPlanError::Pagination)?;
    let sort = match query.sort_by.as_deref() {
        Some(column) => vec![validate_sort_column(column, &query.sort_order)?],
        None => DEFAULT_SORT.to_vec(),
    };
    let table = match query.filters.status_filter.as_deref() {
        Some("deleted") => KeyTable::Deleted,
        Some(_) | None => KeyTable::Live,
    };
    Ok(KeyQueryPlan {
        table,
        where_clause,
        sort,
        skip,
        take: query.size,
    })
}
