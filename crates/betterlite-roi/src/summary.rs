use indexmap::IndexMap;

use crate::records::ROIReport;
use crate::summary::error::SummaryError;
use crate::summary::models::{ROIPersonSummary, ROISummary};

pub mod error;
pub mod identity;
pub mod identity_map;
mod json_number;
mod metrics;
pub mod models;
mod people;
mod pulls;
mod tests;
mod trend;

pub fn summarize(
    report: &ROIReport,
    mappings: &IndexMap<String, String>,
) -> Result<ROISummary, SummaryError> {
    let complete_scope = report.unavailable_repos.as_ref().is_none_or(Vec::is_empty);
    let mut pulls = pulls::pull_summaries(report, mappings)?;
    let mut people = people::group_people(&report.spend, &pulls)
        .into_iter()
        .map(|(key, rows)| people::summarize_person(key, &rows, complete_scope))
        .collect::<Result<Vec<ROIPersonSummary>, _>>()?;
    let trend = trend::daily_trend(&report.spend, &pulls, &people)?;
    let metrics = metrics::summary_metrics(&people, &pulls, complete_scope)?;
    people.sort_by(|left, right| {
        right
            .hours
            .cmp(&left.hours)
            .then_with(|| left.id.cmp(&right.id))
    });
    pulls.sort_by(|left, right| right.merged_at.cmp(&left.merged_at));
    Ok(ROISummary {
        id: report.id.clone(),
        mode: report.mode.clone(),
        start: report.start.clone(),
        end: report.end.clone(),
        synced_at: report.synced_at.clone(),
        repos: report.repos.clone(),
        estimator_model: report.estimator_model.clone(),
        estimator_prompt: report.estimator_prompt.clone(),
        warnings: report.warnings.clone().unwrap_or_default(),
        effort_basis: report.effort_basis.clone(),
        metrics,
        people,
        pulls,
        trend,
    })
}
