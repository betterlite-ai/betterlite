use std::collections::HashMap;
use std::ops::Sub as _;

use crate::baseline_history::cache_plan::CountedBreakpoint;
use crate::baseline_history::error::BaselineHistoryError;
use crate::baseline_history::estimate::{
    entry_ambiguous, entry_matches, is_plan_valid, markers_for,
};
use crate::baseline_history::model::{BaselineHistory, BaselineObservation, CacheEntry, Outcome};
use crate::baseline_history::time::{checked_add_timestamp, ttl_seconds_as_f64};
use crate::baseline_history::usage::is_usage_complete;

pub(crate) const MAX_CACHE_TTL_SECONDS: f64 = 3600.0;

pub(crate) fn writes_for(
    history: &BaselineHistory,
    observation: &BaselineObservation,
) -> Result<Vec<CacheEntry>, BaselineHistoryError> {
    if observation.outcome != Outcome::Complete
        || observation.started_at < history.blocked_until
        || !is_usage_complete(observation.usage.as_ref())
    {
        return Ok(Vec::new());
    }
    let Some(plan) = observation
        .plan
        .as_ref()
        .filter(|plan| is_plan_valid(Some(plan)))
    else {
        return Ok(Vec::new());
    };
    let markers = markers_for(plan, observation.minimum_cache_tokens);
    let ambiguous: Vec<&CacheEntry> = history
        .entries
        .iter()
        .filter(|entry| entry_ambiguous(entry, &markers, observation.started_at))
        .collect();
    let mut entries = Vec::new();
    if let Some(entry) = refresh_entry(history, observation, &markers, &ambiguous)? {
        entries.push(entry);
    }
    entries.extend(marker_entries(observation, &markers, &ambiguous)?);
    Ok(entries)
}

fn refresh_entry(
    history: &BaselineHistory,
    observation: &BaselineObservation,
    markers: &[&CountedBreakpoint],
    ambiguous: &[&CacheEntry],
) -> Result<Option<CacheEntry>, BaselineHistoryError> {
    if !ambiguous.is_empty() {
        return Ok(None);
    }
    let Some(hit) = history
        .entries
        .iter()
        .filter(|entry| !entry.uncertain && entry_matches(entry, markers, observation.started_at))
        .max_by_key(|entry| entry.tokens)
    else {
        return Ok(None);
    };
    if markers
        .iter()
        .any(|marker| marker.fingerprint == hit.fingerprint)
    {
        return Ok(None);
    }
    let ttl = ttl_seconds_as_f64(hit.ttl_seconds)?;
    let expires_at = checked_add_timestamp(observation.started_at, ttl)?;
    Ok(Some(CacheEntry {
        fingerprint: hit.fingerprint.clone(),
        content_fingerprint: hit.content_fingerprint.clone(),
        tokens: hit.tokens,
        ttl_seconds: hit.ttl_seconds,
        available_at: observation.available_at,
        expires_at,
        uncertain: false,
    }))
}

fn marker_entries(
    observation: &BaselineObservation,
    markers: &[&CountedBreakpoint],
    ambiguous: &[&CacheEntry],
) -> Result<Vec<CacheEntry>, BaselineHistoryError> {
    let is_uncertain = !ambiguous.is_empty();
    let mut entries = Vec::with_capacity(markers.len());
    for marker in markers {
        let longest_ttl = ambiguous
            .iter()
            .map(|entry| entry.ttl_seconds)
            .fold(marker.ttl_seconds, |longest, ttl_seconds| {
                longest.max(ttl_seconds)
            });
        let ttl = ttl_seconds_as_f64(longest_ttl)?;
        let expires_at = checked_add_timestamp(observation.started_at, ttl)?;
        entries.push(CacheEntry {
            fingerprint: marker.fingerprint.clone(),
            content_fingerprint: marker.content_fingerprint.clone(),
            tokens: marker.prefix_tokens,
            ttl_seconds: marker.ttl_seconds,
            available_at: observation.available_at,
            expires_at,
            uncertain: is_uncertain,
        });
    }
    Ok(entries)
}

pub(crate) fn compact_entries(entries: &[CacheEntry], started: f64) -> Vec<CacheEntry> {
    let mut groups: HashMap<(String, String, i64, i64, bool), Vec<CacheEntry>> = HashMap::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.expires_at >= started.sub(MAX_CACHE_TTL_SECONDS))
    {
        let key = (
            entry.fingerprint.clone(),
            entry.content_fingerprint.clone(),
            entry.tokens,
            entry.ttl_seconds,
            entry.uncertain,
        );
        groups.entry(key).or_default().push(entry.clone());
    }
    groups
        .into_values()
        .flat_map(|group| retained_from_group(&group, started))
        .collect()
}

fn retained_from_group(group: &[CacheEntry], started: f64) -> Vec<CacheEntry> {
    let mut retained = Vec::new();
    if let Some(entry) = group
        .iter()
        .filter(|entry| entry.available_at <= started)
        .max_by(|left, right| left.expires_at.total_cmp(&right.expires_at))
    {
        retained.push(entry.clone());
    }
    retained.extend(
        group
            .iter()
            .filter(|entry| entry.available_at > started)
            .cloned(),
    );
    retained
}

mod tests;
