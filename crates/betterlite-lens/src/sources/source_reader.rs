use std::error::Error as StdError;

use thiserror::Error;

use crate::models::execution::{Execution, ExecutionContent, Sample};
use crate::models::finding::Evidence;
use crate::models::settings::{LensSettings, Scope};
use crate::sources::activity_availability::ActivityAvailability;
use crate::sources::agent_row::AgentRow;
use crate::sources::count_row::CountRow;
use crate::sources::execution_row::ExecutionRow;
use crate::sources::parameters::parameters;
use crate::sources::parse_execution::ParseExecutionError;
use crate::sources::part_row::PartRow;
use crate::sources::row_decode::{RowsError, decode_rows};
use crate::sources::selection_id::selection_id;
use crate::sources::source_reader_content::{self, CONTENT_PAGE_SIZE};
use crate::sources::source_reader_sample::{self, SampleWindow};
use crate::sources::storage::{ContentParameters, EvidenceParameters, Storage};

pub struct SourceReader<S> {
    pub storage: S,
}

#[derive(Debug, Error)]
pub enum SourceError<E>
where
    E: StdError + 'static,
{
    #[error("lens storage query failed")]
    Query(#[source] E),
    #[error(transparent)]
    Rows(#[from] RowsError),
    #[error(transparent)]
    ExecutionId(#[from] ParseExecutionError),
}

impl<S> SourceReader<S>
where
    S: Storage + Sync,
{
    pub async fn agents(&self, scope: &Scope) -> Result<Vec<String>, SourceError<S::Error>> {
        let bytes = self
            .storage
            .lens_agents(parameters(scope, &[]))
            .await
            .map_err(SourceError::Query)?;
        let rows: Vec<AgentRow> = decode_rows(&bytes)?;
        Ok(rows.into_iter().map(|row| row.agent_name).collect())
    }

    pub async fn availability(
        &self,
        scope: &Scope,
    ) -> Result<ActivityAvailability, SourceError<S::Error>> {
        let bytes = self
            .storage
            .lens_availability(parameters(scope, &[]))
            .await
            .map_err(SourceError::Query)?;
        let rows: Vec<ActivityAvailability> = decode_rows(&bytes)?;
        Ok(rows.into_iter().next().unwrap_or_default())
    }

    pub async fn sample(
        &self,
        scope: &Scope,
        settings: &LensSettings,
        window: SampleWindow,
    ) -> Result<Sample, SourceError<S::Error>> {
        let mut execution_ids = Vec::with_capacity(settings.execution_ids.len());
        for value in &settings.execution_ids {
            execution_ids.push(selection_id(value)?);
        }
        let params = source_reader_sample::sample_params(scope, settings, &window, execution_ids);
        let bytes = self
            .storage
            .lens_sample(params)
            .await
            .map_err(SourceError::Query)?;
        let rows: Vec<ExecutionRow> = decode_rows(&bytes)?;
        let eligible = rows.first().map_or(0, |row| row.eligible);
        let selected = rows.first().map_or(0, |row| row.selected);
        let (next_offset, next_cursor) = source_reader_sample::paging(&rows, &window);
        let executions = rows
            .iter()
            .map(source_reader_sample::execution_from_row)
            .collect();
        Ok(Sample {
            executions,
            eligible,
            selected,
            next_offset,
            next_cursor,
        })
    }

    pub async fn content(
        &self,
        scope: &Scope,
        execution: &Execution,
        cursor: &str,
        offset: i64,
    ) -> Result<ExecutionContent, SourceError<S::Error>> {
        let params = ContentParameters {
            base: parameters(scope, &[]),
            source: execution.source,
            id: execution.trace_id.clone(),
            trace_ref: execution.trace_ref.clone(),
            record_team: execution.team_id.clone(),
            cursor: cursor.to_owned(),
            offset: offset.saturating_add(1),
        };
        let bytes = self
            .storage
            .lens_content(params)
            .await
            .map_err(SourceError::Query)?;
        let rows: Vec<PartRow> = decode_rows(&bytes)?;
        let truncated_any = rows.iter().any(|row| row.truncated != 0);
        let next_cursor = (rows.len() == CONTENT_PAGE_SIZE)
            .then(|| rows.last())
            .flatten()
            .map(|row| row.span_id.clone());
        let parts = source_reader_content::trace_parts(execution, &rows);
        Ok(ExecutionContent {
            execution: execution.clone(),
            parts,
            next_cursor,
            partial: !execution.root_seen || truncated_any,
        })
    }

    pub async fn verify_evidence(
        &self,
        scope: &Scope,
        execution: &Execution,
        evidence: &Evidence,
    ) -> Result<bool, SourceError<S::Error>> {
        let params = EvidenceParameters {
            base: parameters(scope, &[]),
            source: execution.source,
            id: execution.trace_id.clone(),
            trace_ref: execution.trace_ref.clone(),
            record_team: execution.team_id.clone(),
            span: evidence.span_id.clone(),
            quote: evidence.quote.clone(),
        };
        let bytes = self
            .storage
            .lens_evidence(params)
            .await
            .map_err(SourceError::Query)?;
        let rows: Vec<CountRow> = decode_rows(&bytes)?;
        Ok(rows.first().is_some_and(|row| row.count > 0))
    }
}

pub mod agents;
pub mod availability;
