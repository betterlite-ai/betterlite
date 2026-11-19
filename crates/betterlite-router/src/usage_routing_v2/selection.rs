use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use chrono::{DateTime, Utc};
use rand::{Rng, RngExt as _};
use rust_decimal::Decimal;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_value_store::{RouteKey, RouteValue};
use crate::usage_v2_accounting::keys::window_keys;

use super::error::{DeploymentLoad, KeyError, LimitError, NoDeploymentAvailable};

const MODEL: &str = "model";
const RPM: &str = "rpm";
const TPM: &str = "tpm";

// usage_counter_keys: the tpm and the rpm window key of every deployment of the pool, in pool order.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CounterKeys {
    pub tpm: Vec<RouteKey>,
    pub rpm: Vec<RouteKey>,
}

impl CounterKeys {
    pub fn for_deployments(
        deployments: &[DeploymentSpec],
        now: DateTime<Utc>,
    ) -> Result<Self, KeyError> {
        let mut keys = Self::default();
        for deployment in deployments {
            let id = deployment.id().unwrap_or_default();
            let window = window_keys(id, model(deployment), now);
            keys.tpm.push(counter_key(&window.tpm, id)?);
            keys.rpm.push(counter_key(&window.rpm, id)?);
        }
        Ok(keys)
    }
}

fn counter_key(raw: &str, deployment: &str) -> Result<RouteKey, KeyError> {
    RouteKey::parse(raw).map_err(|source| KeyError::TooLong {
        deployment: deployment.to_owned(),
        source,
    })
}

// The pinned key names the deployment by its litellm_params.model, which a built deployment always carries.
fn model(deployment: &DeploymentSpec) -> &str {
    match deployment.litellm_params.get(MODEL) {
        Some(Json::Text(model)) => model,
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            ""
        }
    }
}

// One selection: the healthy pool in pool order and the counter values read for it, aligned with the pool.
pub struct Selection<'pool> {
    pub group: &'pool str,
    pub deployments: &'pool [DeploymentSpec],
    pub input_tokens: i64,
    pub tpm: Option<&'pool [Option<RouteValue>]>,
    pub rpm: Option<&'pool [Option<RouteValue>]>,
}

// One id's row of the pinned counter dicts: a repeated id keeps its first position and its last deployment.
struct Row<'pool> {
    id: &'pool str,
    index: usize,
    tpm: Option<i64>,
    rpm: Option<i64>,
}

// The tpm and rpm limits _return_potential_deployments compares a deployment's usage against.
#[derive(Clone, Copy)]
struct Limits {
    tpm: Option<Decimal>,
    rpm: Option<Decimal>,
}

impl Selection<'_> {
    // _common_checks_available_deployment: lowest absolute tpm under the limits, ties by the caller's randomness.
    pub fn choose<R>(&self, random: &mut R) -> Result<Option<usize>, LimitError>
    where
        R: Rng,
    {
        let mut lowest: Option<Decimal> = None;
        let mut candidates: Vec<usize> = Vec::new();
        for row in self.rows() {
            let Some(deployment) = self.deployments.get(row.index) else {
                continue;
            };
            let limits = Limits {
                tpm: limit(deployment, TPM)?,
                rpm: limit(deployment, RPM)?,
            };
            if self.over_limits(&row, limits) {
                continue;
            }
            // a counter memory did not hold counts as zero, which is what the pinned dict fills it with
            let tpm = Decimal::from(row.tpm.unwrap_or_default());
            match lowest {
                Some(current) if tpm > current => {}
                Some(current) if tpm == current => candidates.push(row.index),
                Some(_) | None => {
                    lowest = Some(tpm);
                    candidates = vec![row.index];
                }
            }
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let index = random.random_range(0..candidates.len());
        Ok(candidates.get(index).copied())
    }

    // The dict async_get_available_deployments reports in its RateLimitError, in first-observation order.
    pub(super) fn no_deployment(&self) -> Result<NoDeploymentAvailable, LimitError> {
        let mut loads: Vec<DeploymentLoad> = Vec::with_capacity(self.deployments.len());
        for (position, deployment) in self.deployments.iter().enumerate() {
            let id = deployment.id().unwrap_or_default();
            let load = DeploymentLoad {
                id: id.to_owned(),
                current_tpm: current(self.tpm, position),
                tpm_limit: limit(deployment, TPM)?,
                current_rpm: current(self.rpm, position),
                rpm_limit: limit(deployment, RPM)?,
            };
            match loads.iter_mut().find(|existing| existing.id == id) {
                Some(existing) => *existing = load,
                None => loads.push(load),
            }
        }
        Ok(NoDeploymentAvailable {
            model: self.group.to_owned(),
            deployments: loads,
        })
    }

    fn rows(&self) -> Vec<Row<'_>> {
        let mut rows: Vec<Row<'_>> = Vec::with_capacity(self.deployments.len());
        for (position, deployment) in self.deployments.iter().enumerate() {
            let id = deployment.id().unwrap_or_default();
            let tpm = read(self.tpm, position);
            let rpm = read(self.rpm, position);
            match rows.iter_mut().find(|row| row.id == id) {
                Some(row) => {
                    row.index = position;
                    row.tpm = tpm;
                    row.rpm = rpm;
                }
                None => rows.push(Row {
                    id,
                    index: position,
                    tpm,
                    rpm,
                }),
            }
        }
        rows
    }

    // item_tpm + input_tokens > tpm, or item_rpm + 1 >= rpm while the rpm counter was read.
    fn over_limits(&self, row: &Row<'_>, limits: Limits) -> bool {
        let tokens = Decimal::from(row.tpm.unwrap_or_default())
            .saturating_add(Decimal::from(self.input_tokens));
        let requests = row
            .rpm
            .map(|requests| Decimal::from(requests).saturating_add(Decimal::ONE));
        limits.tpm.is_some_and(|limit| tokens > limit)
            || limits
                .rpm
                .is_some_and(|limit| requests.is_some_and(|count| count >= limit))
    }
}

// The pinned `or` chain: own limit, litellm_params, model_info, then inf; an explicit zero is used as given.
fn limit(deployment: &DeploymentSpec, field: &'static str) -> Result<Option<Decimal>, LimitError> {
    for source in [
        &deployment.extra,
        &deployment.litellm_params,
        &deployment.model_info,
    ] {
        match source.get(field) {
            None | Some(Json::Null) => {}
            Some(value) => return Ok(Some(amount(deployment, value, field)?)),
        }
    }
    Ok(None)
}

fn amount(
    deployment: &DeploymentSpec,
    value: &Json,
    field: &'static str,
) -> Result<Decimal, LimitError> {
    let Json::Number(number) = value else {
        return Err(non_numeric(deployment, field));
    };
    parse_decimal_lexeme(&number.to_string()).map_err(|_source| non_numeric(deployment, field))
}

fn non_numeric(deployment: &DeploymentSpec, field: &'static str) -> LimitError {
    LimitError::NonNumeric {
        deployment: deployment.id().unwrap_or_default().to_owned(),
        field,
    }
}

// tpm_values[index] if tpm_values else 0, with a value the dict holds as None staying None.
fn current(values: Option<&[Option<RouteValue>]>, position: usize) -> Option<i64> {
    match values {
        Some([]) | None => Some(0),
        Some(list) => read(Some(list), position),
    }
}

fn read(values: Option<&[Option<RouteValue>]>, position: usize) -> Option<i64> {
    let value = values.and_then(|list| list.get(position))?;
    match value {
        Some(RouteValue::Counter(count)) => Some(*count),
        Some(
            RouteValue::Spend(_)
            | RouteValue::Pin(_)
            | RouteValue::SchedulerTuple(_)
            | RouteValue::Members(_),
        )
        | None => None,
    }
}
