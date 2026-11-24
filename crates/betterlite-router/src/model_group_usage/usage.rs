use std::time::SystemTime;

use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::clock::minute_label;
use crate::io_token_reservations::limits::{IoDeployment, IoLimits, minute_keys};
use crate::model_rate_limits::deployment::{model, model_id};
use crate::router_observations::keys::usage_keys;

use super::counters::UsageCounterCache;

// Which RouterCacheEnum series a read covers: the tpm/rpm pair, or the itpm/otpm one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageSeries {
    Standard,
    IoToken,
}

// Router.get_model_group_usage and Router.get_model_group_io_token_usage: one batch read of the first series' keys then the second's, split at the first list's length and summed in read order.
pub async fn group_counter_totals<C>(
    series: UsageSeries,
    model_list: Option<&[DeploymentSpec]>,
    cache: &C,
    now: SystemTime,
) -> Result<(Option<i64>, Option<i64>), C::Error>
where
    C: UsageCounterCache,
{
    let Some(deployments) = model_list else {
        return Ok((None, None));
    };
    let moment = DateTime::<Utc>::from(now);
    let minute = minute_label(now);
    let mut first_keys = Vec::with_capacity(deployments.len());
    let mut second_keys = Vec::with_capacity(deployments.len());
    for deployment in deployments {
        let Some((first, second)) = series_keys(series, deployment, moment, &minute) else {
            continue;
        };
        first_keys.push(first);
        second_keys.push(second);
    }
    let first_count = first_keys.len();
    let mut combined = first_keys;
    combined.extend(second_keys);
    Ok(split_totals(cache.batch_get(&combined).await?, first_count))
}

// One deployment's two keys for the series, or none when its model id or backend model is missing.
fn series_keys(
    series: UsageSeries,
    deployment: &DeploymentSpec,
    moment: DateTime<Utc>,
    minute: &str,
) -> Option<(String, String)> {
    match series {
        UsageSeries::Standard => {
            let keys = usage_keys(model_id(deployment)?, model(deployment)?, moment);
            Some((keys.tpm, keys.rpm))
        }
        UsageSeries::IoToken => {
            let target = IoDeployment {
                model_id: model_id(deployment),
                model: model(deployment),
                limits: IoLimits::default(),
            };
            let keys = minute_keys(&target, minute)?;
            Some((keys.itpm, keys.otpm))
        }
    }
}

// Python's slice: a short answer keeps its source order and never regroups, and no answer sums to none on both sides.
fn split_totals(values: Option<Vec<Option<Json>>>, first_len: usize) -> (Option<i64>, Option<i64>) {
    let Some(answers) = values else {
        return (None, None);
    };
    let split = first_len.min(answers.len());
    let (first, second) = answers.split_at(split);
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
    let observed = value?;
    match observed {
        Json::Bool(flag) => Some(i64::from(u8::from(*flag))),
        Json::Number(number) => match number.as_i64() {
            Some(signed) => Some(signed),
            None => i64::try_from(number.as_u64()?).ok(),
        },
        Json::Text(_) | Json::List(_) | Json::Object(_) | Json::Null => None,
    }
}
