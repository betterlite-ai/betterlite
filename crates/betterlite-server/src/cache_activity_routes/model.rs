use std::ops::{Div, Mul};

use num_traits::ToPrimitive as _;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct CacheActivityGroup {
    pub call_type: String,
    pub api_requests: i64,
    pub cache_hits: i64,
    pub failed_requests: i64,
    pub cached_completion_tokens: i64,
    pub generated_completion_tokens: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CacheActivityTotals {
    pub api_requests: i64,
    pub cache_hits: i64,
    pub failed_requests: i64,
    pub cached_completion_tokens: i64,
    pub cache_hit_ratio: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CacheActivityFilterOptions {
    pub key_aliases: Vec<String>,
    pub models: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CacheActivityErrorBucket {
    pub call_type: String,
    pub error_code: String,
    pub error_class: String,
    pub count: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CacheActivityResponse {
    pub groups: Vec<CacheActivityGroup>,
    pub totals: CacheActivityTotals,
    pub filter_options: CacheActivityFilterOptions,
    pub error_breakdown: Vec<CacheActivityErrorBucket>,
}

#[derive(Clone, Debug, Default)]
pub struct CacheActivityRows {
    pub groups: Vec<CacheActivityGroup>,
    pub error_breakdown: Vec<CacheActivityErrorBucket>,
    pub key_aliases: Vec<String>,
    pub models: Vec<String>,
}

impl CacheActivityResponse {
    #[must_use]
    pub fn new(rows: CacheActivityRows) -> Self {
        let totals = compute_totals(&rows.groups);
        Self {
            groups: rows.groups,
            totals,
            filter_options: CacheActivityFilterOptions {
                key_aliases: rows.key_aliases,
                models: rows.models,
            },
            error_breakdown: rows.error_breakdown,
        }
    }
}

#[must_use]
pub fn compute_totals(groups: &[CacheActivityGroup]) -> CacheActivityTotals {
    let api_requests: i64 = groups.iter().map(|group| group.api_requests).sum();
    let cache_hits: i64 = groups.iter().map(|group| group.cache_hits).sum();
    let failed_requests: i64 = groups.iter().map(|group| group.failed_requests).sum();
    let cached_completion_tokens: i64 = groups
        .iter()
        .map(|group| group.cached_completion_tokens)
        .sum();
    let all_requests = api_requests
        .saturating_add(cache_hits)
        .saturating_add(failed_requests);
    CacheActivityTotals {
        api_requests,
        cache_hits,
        failed_requests,
        cached_completion_tokens,
        cache_hit_ratio: cache_hit_ratio(cache_hits, all_requests),
    }
}

fn cache_hit_ratio(cache_hits: i64, all_requests: i64) -> f64 {
    if all_requests <= 0_i64 {
        return 0.0_f64;
    }
    cache_hits
        .to_f64()
        .zip(all_requests.to_f64())
        .map_or(0.0_f64, |(hits, total)| {
            Mul::mul(Div::div(hits, total), 100.0_f64)
        })
}
