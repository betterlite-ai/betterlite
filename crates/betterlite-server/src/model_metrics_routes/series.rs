use std::collections::BTreeMap;
use std::collections::BTreeSet;

use indexmap::IndexMap;

use super::store::{DailyRow, ExceptionRow, LatencyRow, SameDayRow, SlowRow};
use super::wire::{
    ExceptionDataRow, ExceptionResponse, SeriesResponse, SeriesRow, SlowResponseRow,
};

const DAY_FORMAT: &str = "%Y-%m-%d";
const OPENAI_MARKER: &str = "/openai/";
const HTTPS_MARKER: &str = "https://";

#[derive(Default)]
struct Series {
    entries: BTreeMap<String, BTreeMap<String, f64>>,
    names: BTreeSet<String>,
}

impl Series {
    fn insert(&mut self, key: String, api_base: Option<&str>, model: &str, value: f64) {
        let chosen = api_base
            .filter(|base| base.contains(HTTPS_MARKER))
            .unwrap_or(model);
        let name = without_openai_suffix(chosen);
        self.names.insert(name.clone());
        self.entries.entry(key).or_default().insert(name, value);
    }

    fn finish(self) -> SeriesResponse {
        SeriesResponse {
            data: self
                .entries
                .into_iter()
                .map(|(date, series)| SeriesRow { date, series })
                .collect(),
            all_api_bases: self.names,
        }
    }
}

#[derive(Default)]
struct ModelExceptionCounts {
    total: i64,
    counts: BTreeMap<String, i64>,
}

fn without_openai_suffix(name: &str) -> String {
    name.split_once(OPENAI_MARKER)
        .map_or_else(|| name.to_owned(), |(head, _rest)| head.to_owned())
}

pub(super) fn same_day(rows: &[SameDayRow]) -> SeriesResponse {
    let mut series = Series::default();
    for row in rows {
        series.insert(
            row.request_id.clone(),
            row.api_base.as_deref(),
            &row.model,
            row.time_to_first_token,
        );
    }
    series.finish()
}

pub(super) fn daily(rows: &[DailyRow]) -> SeriesResponse {
    let mut series = Series::default();
    for row in rows {
        series.insert(
            row.day.format(DAY_FORMAT).to_string(),
            row.api_base.as_deref(),
            &row.model,
            row.time_to_first_token,
        );
    }
    series.finish()
}

pub(super) fn latency(rows: &[LatencyRow]) -> SeriesResponse {
    let mut series = Series::default();
    for row in rows {
        series.insert(
            row.day.format(DAY_FORMAT).to_string(),
            row.api_base.as_deref(),
            &row.model,
            row.avg_latency_per_token,
        );
    }
    series.finish()
}

pub(super) fn slow(rows: &[SlowRow]) -> Vec<SlowResponseRow> {
    rows.iter()
        .map(|row| SlowResponseRow {
            api_base: without_openai_suffix(row.api_base.as_deref().unwrap_or_default()),
            total_count: row.total_count,
            slow_count: row.slow_count,
        })
        .collect()
}

pub(super) fn exceptions(rows: &[ExceptionRow]) -> ExceptionResponse {
    let mut grouped: IndexMap<String, ModelExceptionCounts> = IndexMap::new();
    let mut exception_types = BTreeSet::new();
    for row in rows {
        exception_types.insert(row.exception_type.clone());
        let counts = grouped.entry(row.model.clone()).or_default();
        counts.counts.insert(row.exception_type.clone(), row.count);
        counts.total = counts.total.saturating_add(1);
    }
    ExceptionResponse {
        data: grouped
            .into_iter()
            .map(|(model, counts)| ExceptionDataRow {
                model,
                total_exceptions: counts.total,
                exception_counts: counts.counts,
            })
            .collect(),
        exception_types,
    }
}
