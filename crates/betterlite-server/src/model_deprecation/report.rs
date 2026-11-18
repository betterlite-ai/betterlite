use chrono::{DateTime, NaiveDate, Utc};

use super::catalog::DeprecationCatalog;
use super::clock::UtcClock;
use super::resolve::{Deployment, resolve_deprecation};
use super::types::{DeprecationStatus, ModelDeprecationInfo, ModelDeprecationResponse};

#[must_use]
pub fn collect_model_deprecations(
    deployments: &[Deployment<'_>],
    catalog: &dyn DeprecationCatalog,
    clock: &dyn UtcClock,
    warn_within_days: i32,
    today: Option<NaiveDate>,
) -> ModelDeprecationResponse {
    let checked_at = clock.now();
    let effective_today = today.unwrap_or_else(|| checked_at.date_naive());
    let models: Vec<ModelDeprecationInfo> = deployments
        .iter()
        .filter_map(|deployment| build_info(deployment, effective_today, warn_within_days, catalog))
        .collect();
    let deduped = dedupe(models);
    let dated = deduped.len();
    let total = deployments.len();
    tracing::debug!("model_deprecation: {dated}/{total} deployments carry a deprecation date");
    bucket(deduped, warn_within_days, checked_at)
}

fn build_info(
    deployment: &Deployment<'_>,
    today: NaiveDate,
    warn_within_days: i32,
    catalog: &dyn DeprecationCatalog,
) -> Option<ModelDeprecationInfo> {
    let model_name = deployment.model_name.filter(|name| !name.is_empty())?;
    let resolved = resolve_deprecation(deployment, catalog)?;
    let days_until = resolved
        .deprecation_date
        .signed_duration_since(today)
        .num_days();
    Some(ModelDeprecationInfo {
        model_name: model_name.to_owned(),
        litellm_model: resolved.litellm_model,
        deprecation_date: resolved.deprecation_date,
        days_until_deprecation: days_until,
        status: classify(days_until, warn_within_days),
        litellm_provider: resolved.litellm_provider,
    })
}

#[must_use]
pub const fn classify(days_until: i64, warn_within_days: i32) -> DeprecationStatus {
    if days_until < 0 {
        return DeprecationStatus::Deprecated;
    }
    if days_until <= i64::from(warn_within_days) {
        return DeprecationStatus::Imminent;
    }
    DeprecationStatus::Upcoming
}

fn dedupe(mut models: Vec<ModelDeprecationInfo>) -> Vec<ModelDeprecationInfo> {
    models.sort_by(|left, right| {
        left.model_name
            .cmp(&right.model_name)
            .then(left.deprecation_date.cmp(&right.deprecation_date))
    });
    let mut unique: Vec<ModelDeprecationInfo> = Vec::new();
    for model in models {
        let repeated = unique.last().is_some_and(|last| {
            last.model_name == model.model_name && last.deprecation_date == model.deprecation_date
        });
        if !repeated {
            unique.push(model);
        }
    }
    unique
}

fn bucket(
    mut models: Vec<ModelDeprecationInfo>,
    warn_within_days: i32,
    checked_at: DateTime<Utc>,
) -> ModelDeprecationResponse {
    models.sort_by(|left, right| left.deprecation_date.cmp(&right.deprecation_date));
    let mut response = ModelDeprecationResponse {
        deprecated: Vec::new(),
        imminent: Vec::new(),
        upcoming: Vec::new(),
        warn_within_days,
        checked_at,
    };
    for model in models {
        match model.status {
            DeprecationStatus::Deprecated => response.deprecated.push(model),
            DeprecationStatus::Imminent => response.imminent.push(model),
            DeprecationStatus::Upcoming => response.upcoming.push(model),
        }
    }
    response
}
