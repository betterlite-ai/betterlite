use crate::models::execution::{Execution, ExecutionSource};
use crate::models::settings::{LensSettings, MetadataFilter, Scope};
use crate::sources::execution_id::execution_id;
use crate::sources::execution_row::ExecutionRow;
use crate::sources::parameters::parameters;
use crate::sources::storage::SampleParameters;

// A metadata attribute this specific key marks as internal and never surfaced on an Execution.
const HIDDEN_METADATA_KEY: &str = "litellm.api_key_hash";

pub struct SampleWindow {
    pub start: i64,
    pub end: i64,
    pub offset: i64,
    pub page_size: usize,
    pub preview: bool,
    pub cursor: String,
}

const fn execution_source_str(source: ExecutionSource) -> &'static str {
    match source {
        ExecutionSource::Traces => "traces",
        ExecutionSource::Requests => "requests",
    }
}

fn metadata_visible(key: &str, value: &str) -> bool {
    if key == HIDDEN_METADATA_KEY {
        return false;
    }
    (1..=200).contains(&key.chars().count()) && (1..=500).contains(&value.chars().count())
}

pub(crate) fn execution_from_row(row: &ExecutionRow) -> Execution {
    let metadata = row
        .attributes
        .iter()
        .filter(|(key, value)| metadata_visible(key, value))
        .map(|(key, value)| MetadataFilter {
            key: key.clone(),
            value: value.clone(),
        })
        .collect();
    Execution {
        id: execution_id(
            execution_source_str(row.source),
            &row.team_id,
            &row.trace_id,
            &row.trace_ref,
        ),
        source: row.source,
        trace_id: row.trace_id.clone(),
        trace_ref: row.trace_ref.clone(),
        team_id: row.team_id.clone(),
        name: row.name.clone(),
        start_time: row.start_time.clone(),
        span_count: row.span_count,
        root_seen: row.root_seen != 0,
        service: row.service.clone(),
        metadata,
    }
}

pub(crate) fn sample_params(
    scope: &Scope,
    settings: &LensSettings,
    window: &SampleWindow,
    execution_ids: Vec<String>,
) -> SampleParameters {
    let base = parameters(scope, &settings.filters);
    SampleParameters {
        base,
        source: settings.source,
        start: window.start,
        end: window.end,
        service: settings.service.clone(),
        agent_name: settings.agent_name.clone(),
        limit: window.page_size,
        offset: window.offset,
        after: window.cursor.clone(),
        sample_percent: settings.sample_percent,
        sample_cap: settings.sample_size.unwrap_or(0),
        preview: window.preview,
        selected_team: settings.team_id.clone(),
        execution_ids,
    }
}

pub(crate) fn paging(
    rows: &[ExecutionRow],
    window: &SampleWindow,
) -> (Option<i64>, Option<String>) {
    let eligible = rows.first().map_or(0, |row| row.eligible);
    let selected = rows.first().map_or(0, |row| row.selected);
    let boundary = if window.preview { eligible } else { selected };
    let row_count = i64::try_from(rows.len()).unwrap_or(i64::MAX);
    let next_cursor = (rows.len() == window.page_size)
        .then(|| rows.last())
        .flatten()
        .map(|row| row.selection_key.clone());
    let next_offset = (window.page_size != 0
        && !rows.is_empty()
        && window.offset.saturating_add(row_count) < boundary)
        .then(|| window.offset.saturating_add(row_count));
    (next_offset, next_cursor)
}

mod tests;
