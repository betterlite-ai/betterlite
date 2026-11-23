use std::time::SystemTime;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::clock::minute_label;
use crate::io_token_reservations::limits::{IoDeployment, IoLimits, minute_keys};
use crate::model_rate_limits::deployment::{model, model_id};
use crate::router_observations::keys::usage_keys;

use super::counters::UsageCounterCache;

// Router.get_model_group_usage's answer: None when no counter was observed, a sum over the observed ones.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub tpm: Option<i64>,
    pub rpm: Option<i64>,
}

// Router.get_model_group_io_token_usage's answer, summed the same way over the itpm and otpm keys.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IoUsage {
    pub itpm: Option<i64>,
    pub otpm: Option<i64>,
}

// Every deployment's tpm keys then every rpm key, in one batch read, summed back per side.
pub async fn get_model_group_usage<C>(
    model_list: Option<&[DeploymentSpec]>,
    cache: &C,
    now: SystemTime,
) -> Result<Usage, C::Error>
where
    C: UsageCounterCache,
{
    let Some(deployments) = model_list else {
        return Ok(Usage::default());
    };
    let moment = DateTime::<Utc>::from(now);
    let mut tpm_keys = Vec::with_capacity(deployments.len());
    let mut rpm_keys = Vec::with_capacity(deployments.len());
    for deployment in deployments {
        let (Some(id), Some(litellm_model)) = (model_id(deployment), model(deployment)) else {
            continue;
        };
        let keys = usage_keys(id, litellm_model, moment);
        tpm_keys.push(keys.tpm);
        rpm_keys.push(keys.rpm);
    }
    let tpm_count = tpm_keys.len();
    let mut combined = tpm_keys;
    combined.extend(rpm_keys);
    let (tpm, rpm) = split_totals(cache.batch_get(&combined).await?, tpm_count);
    Ok(Usage { tpm, rpm })
}

// The io-token twin: every itpm key then every otpm key, in the same one read and slice.
pub async fn get_model_group_io_token_usage<C>(
    model_list: Option<&[DeploymentSpec]>,
    cache: &C,
    now: SystemTime,
) -> Result<IoUsage, C::Error>
where
    C: UsageCounterCache,
{
    let Some(deployments) = model_list else {
        return Ok(IoUsage::default());
    };
    let minute = minute_label(now);
    let mut itpm_keys = Vec::with_capacity(deployments.len());
    let mut otpm_keys = Vec::with_capacity(deployments.len());
    for deployment in deployments {
        let target = IoDeployment {
            model_id: model_id(deployment),
            model: model(deployment),
            limits: IoLimits::default(),
        };
        let Some(keys) = minute_keys(&target, &minute) else {
            continue;
        };
        itpm_keys.push(keys.itpm);
        otpm_keys.push(keys.otpm);
    }
    let itpm_count = itpm_keys.len();
    let mut combined = itpm_keys;
    combined.extend(otpm_keys);
    let (itpm, otpm) = split_totals(cache.batch_get(&combined).await?, itpm_count);
    Ok(IoUsage { itpm, otpm })
}

// Python's slice: a short answer keeps its source order and never regroups, and no answer sums to none on both sides.
fn split_totals(values: Option<Vec<Option<Json>>>, first_len: usize) -> (Option<i64>, Option<i64>) {
    let Some(values) = values else {
        return (None, None);
    };
    let split = first_len.min(values.len());
    let (first, second) = values.split_at(split);
    (total(first), total(second))
}

// isinstance(value, int) with bool accepted: every other JSON shape, a float included, is skipped.
fn total(values: &[Option<Json>]) -> Option<i64> {
    let mut total: Option<i64> = None;
    for value in values {
        let Some(number) = counter(value.as_ref()) else {
            continue;
        };
        total = Some(total.unwrap_or(0).saturating_add(number));
    }
    total
}

fn counter(value: Option<&Json>) -> Option<i64> {
    match value {
        Some(Json::Bool(flag)) => Some(i64::from(u8::from(*flag))),
        Some(Json::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|count| i64::try_from(count).ok())),
        Some(Json::Text(_) | Json::List(_) | Json::Object(_) | Json::Null) | None => None,
    }
}
