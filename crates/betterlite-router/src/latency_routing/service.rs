use betterlite_models::decisions::Json;
use rand::{Rng, RngExt as _};

use crate::deployment_identity::spec::DeploymentSpec;
use crate::latency_observations::samples::{LatencySamples, ModelGroupSamples};
use crate::latency_rank_math;
use crate::latency_sample_math;
use crate::router_core::error::RoutingError;

// the one zero LiteLLM seeds an unused deployment's latency window with
const UNSEEN_LATENCY: [f64; 1] = [0.0_f64];

// RoutingArgs: the two lowest-latency arguments a selection reads, both defaulted by the handler
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoutingArgs {
    pub lowest_latency_buffer: f64,
    pub ttft_percentile: Option<f64>,
}

impl Default for RoutingArgs {
    fn default() -> Self {
        Self {
            lowest_latency_buffer: 0.0_f64,
            ttft_percentile: None,
        }
    }
}

// One selection: the healthy pool, the model group's stored windows, and the request's own inputs.
pub struct Selection<'request> {
    pub deployments: &'request [DeploymentSpec],
    pub samples: &'request ModelGroupSamples,
    pub stream: bool,
    pub input_tokens: i64,
    // the "%Y-%m-%d-%H-%M" bucket the minute counters are read from
    pub minute: &'request str,
}

// What one selection reads from the pool: every score in pool order, then the eligible positions lowest first and the buffer's prefix of them
#[derive(Clone, Debug, PartialEq)]
pub struct Ranking {
    pub scores: Vec<f64>,
    pub candidates: Vec<usize>,
    pub valid: Vec<usize>,
}

impl Ranking {
    // random.choice(valid_deployments): a uniform pick among the buffered positions, absent when none fit
    pub fn choose<R>(&self, random: &mut R) -> Option<usize>
    where
        R: Rng,
    {
        let count = self.valid.len();
        if count == 0 {
            return None;
        }
        self.valid.get(random.random_range(0..count)).copied()
    }
}

// LowestLatencyLoggingHandler._get_available_deployments: the latency strategy's rank and pick.
pub struct LatencyRouting {
    pub args: RoutingArgs,
}

impl LatencyRouting {
    // the ranked pool: scores for every deployment, then the eligible ones the buffer keeps
    pub fn rank(&self, selection: &Selection<'_>) -> Result<Ranking, RoutingError> {
        let mut scores = Vec::with_capacity(selection.deployments.len());
        let mut eligible = Vec::new();
        for (position, deployment) in selection.deployments.iter().enumerate() {
            let stored = deployment
                .id()
                .and_then(|id| selection.samples.deployments.get(id));
            let score = self.score(stored, selection.stream);
            if available(deployment, stored, selection)? {
                eligible.push((position, score));
            }
            scores.push(score);
        }
        // sorted() is stable, so equal scores keep the pool's order
        eligible.sort_by(|left, right| left.1.total_cmp(&right.1));
        let ranked: Vec<f64> = eligible.iter().map(|(_, score)| *score).collect();
        let kept = latency_rank_math::valid_count(&ranked, self.args.lowest_latency_buffer);
        let candidates: Vec<usize> = eligible.iter().map(|(position, _)| *position).collect();
        let valid = candidates.iter().copied().take(kept).collect();
        Ok(Ranking {
            scores,
            candidates,
            valid,
        })
    }

    // _average_latency and _percentile_latency: streaming reads the TTFT window while it has samples
    fn score(&self, stored: Option<&LatencySamples>, stream: bool) -> f64 {
        let empty: &[f64] = &[];
        let latency = stored.map_or(UNSEEN_LATENCY.as_slice(), |samples| {
            samples.latency.as_slice()
        });
        let first_token = stored.map_or(empty, |samples| {
            samples.time_to_first_token_seconds.as_slice()
        });
        if !stream || first_token.is_empty() {
            return latency_sample_math::average(latency);
        }
        let Some(percentile) = self.args.ttft_percentile else {
            return latency_sample_math::average(first_token);
        };
        latency_sample_math::percentile(first_token, percentile)
            .unwrap_or_else(|| latency_sample_math::average(first_token))
    }
}

// item_tpm + input_tokens > tpm, or item_rpm + 1 > rpm, leaves the deployment out of the pool
fn available(
    deployment: &DeploymentSpec,
    stored: Option<&LatencySamples>,
    selection: &Selection<'_>,
) -> Result<bool, RoutingError> {
    let usage = stored
        .and_then(|samples| samples.minutes.get(selection.minute))
        .copied()
        .unwrap_or_default();
    let tokens = i128::from(usage.tpm).saturating_add(i128::from(selection.input_tokens));
    let requests = i128::from(usage.rpm).saturating_add(1);
    Ok(tokens <= limit(deployment, "tpm")?.unwrap_or(i128::MAX)
        && requests <= limit(deployment, "rpm")?.unwrap_or(i128::MAX))
}

// _deployment.get("tpm"), then litellm_params.tpm, then model_info.tpm; a null or absent limit is unbounded
fn limit(deployment: &DeploymentSpec, field: &'static str) -> Result<Option<i128>, RoutingError> {
    for source in [
        &deployment.extra,
        &deployment.litellm_params,
        &deployment.model_info,
    ] {
        match source.get(field) {
            None | Some(Json::Null) => {}
            Some(Json::Number(number)) => {
                return number
                    .as_i128()
                    .map(Some)
                    .ok_or(RoutingError::Metric { field });
            }
            Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
                return Err(RoutingError::Metric { field });
            }
        }
    }
    Ok(None)
}
