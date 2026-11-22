use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct SeriesResponse {
    pub(super) data: Vec<SeriesRow>,
    pub(super) all_api_bases: BTreeSet<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct SeriesRow {
    pub(super) date: String,
    #[serde(flatten)]
    pub(super) series: BTreeMap<String, f64>,
}

#[derive(Debug, Serialize)]
pub(super) struct ExceptionResponse {
    pub(super) data: Vec<ExceptionDataRow>,
    pub(super) exception_types: BTreeSet<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ExceptionDataRow {
    pub(super) model: String,
    pub(super) total_exceptions: i64,
    #[serde(flatten)]
    pub(super) exception_counts: BTreeMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub(super) struct SlowResponseRow {
    pub(super) api_base: String,
    pub(super) total_count: i64,
    pub(super) slow_count: i64,
}
