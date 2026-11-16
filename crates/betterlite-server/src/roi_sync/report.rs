use std::collections::BTreeMap;

use betterlite_roi::estimator::json::JsonError;
use betterlite_roi::records::{ROIPullRecord, ROIReport, ROISpendRecord};
use betterlite_roi::settings::ROISettings;
use chrono::NaiveDate;
use thiserror::Error;

use crate::roi_cache::key::settings_fingerprint;
use crate::roi_cache::repositories::RepositoryBatch;

#[derive(Debug, Error)]
pub(crate) enum ReportError {
    #[error("the settings fingerprint could not be computed")]
    Fingerprint(#[source] JsonError),
    #[error("pull request {position} is missing from the analysis results")]
    MissingPull { position: usize },
}

pub(crate) struct ReportParts<'run> {
    pub(crate) settings: &'run ROISettings,
    pub(crate) start: NaiveDate,
    pub(crate) end: NaiveDate,
    pub(crate) synced_at: String,
    pub(crate) spend: Vec<ROISpendRecord>,
    pub(crate) processed: BTreeMap<usize, ROIPullRecord>,
    pub(crate) pull_count: usize,
    pub(crate) repositories: &'run RepositoryBatch,
}

pub(crate) fn build_report(parts: ReportParts<'_>) -> Result<ROIReport, ReportError> {
    let fingerprint = settings_fingerprint(parts.settings).map_err(ReportError::Fingerprint)?;
    let mut pulls = Vec::with_capacity(parts.pull_count);
    for position in 0..parts.pull_count {
        let record = parts
            .processed
            .get(&position)
            .cloned()
            .ok_or(ReportError::MissingPull { position })?;
        pulls.push(record);
    }
    Ok(ROIReport {
        mode: "live".to_owned(),
        start: parts.start.to_string(),
        end: parts.end.to_string(),
        synced_at: parts.synced_at,
        repos: parts.settings.repos.clone(),
        estimator_model: parts.settings.estimator_model.clone(),
        estimator_prompt: parts.settings.estimator_prompt.clone(),
        effort_basis: "without_ai".to_owned(),
        spend: parts.spend,
        pulls,
        settings_fingerprint: fingerprint,
        warnings: Some(parts.repositories.warnings.clone()),
        unavailable_repos: Some(parts.repositories.unavailable_repos.clone()),
        id: None,
    })
}
