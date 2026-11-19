use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::cost_routing::keys::minute_label;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::latency_observations::samples::{
    LatencySamples, MinuteUsage, ModelGroupSamples,
};
use betterlite_router::latency_routing::service::{
    LatencyRouting, Ranking, RoutingArgs, Selection,
};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::latency_observations::Seconds;
use crate::router_probes::random::InjectedRandom;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub model_group: String,
    pub settings: SettingsInput,
    pub now_millis: u64,
    pub stream: bool,
    pub input_tokens: i64,
    pub deployments: Vec<DeploymentInput>,
    pub stored: Vec<StoredInput>,
}

// RoutingArgs of the lowest-latency handler the Python probe builds
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub lowest_latency_buffer: Seconds,
    #[serde(default)]
    pub ttft_percentile: Option<Seconds>,
}

// one healthy deployment: its id, and the tpm/rpm limits its litellm_params carry
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentInput {
    pub id: String,
    #[serde(default)]
    pub tpm: Option<i64>,
    #[serde(default)]
    pub rpm: Option<i64>,
}

// one entry of the model group's stored map, keyed by the deployment id the callback recorded it under
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredInput {
    pub id: String,
    #[serde(default)]
    pub latency: Vec<Seconds>,
    #[serde(default)]
    pub time_to_first_token_seconds: Vec<Seconds>,
    #[serde(default)]
    pub minutes: BTreeMap<String, MinuteUsage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    pub selected: Option<String>,
    pub valid: Vec<String>,
    pub scores: Option<BTreeMap<String, Seconds>>,
}

pub(crate) fn run(
    input: &Input,
    random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let deployments = build_deployments(input)?;
    let samples = build_samples(input);
    let minute = minute_of(input.now_millis)?;
    let router = LatencyRouting {
        args: RoutingArgs {
            lowest_latency_buffer: input.settings.lowest_latency_buffer.0,
            ttft_percentile: input
                .settings
                .ttft_percentile
                .map(|percentile| percentile.0),
        },
    };
    let selection = Selection {
        deployments: &deployments,
        samples: &samples,
        stream: input.stream,
        input_tokens: input.input_tokens,
        minute: &minute,
    };
    let ranking = router
        .rank(&selection)
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
    Ok(ProbeOutput::LatencyRouting(report(
        &deployments,
        &ranking,
        random,
    )))
}

// LiteLLM writes the score map only after its pick, so a selection without a deployment reports none
fn report(
    deployments: &[DeploymentSpec],
    ranking: &Ranking,
    random: &mut InjectedRandom<'_>,
) -> Output {
    let selected = ranking
        .choose(random)
        .and_then(|position| deployments.get(position))
        .and_then(DeploymentSpec::id)
        .map(str::to_owned);
    let mut valid: Vec<String> = ranking
        .valid
        .iter()
        .filter_map(|position| deployments.get(*position))
        .filter_map(DeploymentSpec::id)
        .map(str::to_owned)
        .collect();
    valid.sort();
    let scores = selected
        .as_ref()
        .map(|_picked| scores_by_deployment(deployments, ranking));
    Output {
        selected,
        valid,
        scores,
    }
}

fn scores_by_deployment(
    deployments: &[DeploymentSpec],
    ranking: &Ranking,
) -> BTreeMap<String, Seconds> {
    let mut scores = BTreeMap::new();
    for (deployment, score) in deployments.iter().zip(&ranking.scores) {
        if let Some(id) = deployment.id() {
            scores.insert(id.to_owned(), Seconds(*score));
        }
    }
    scores
}

fn build_deployments(input: &Input) -> Result<Vec<DeploymentSpec>, OperationError> {
    let mut deployments = Vec::with_capacity(input.deployments.len());
    for entry in &input.deployments {
        let mut params = JsonObject::from_iter([
            ("model".to_owned(), Json::Text("openai/probe".to_owned())),
            ("api_base".to_owned(), Json::Text(entry.id.clone())),
        ]);
        for (field, value) in [("tpm", entry.tpm), ("rpm", entry.rpm)] {
            if let Some(limit) = value {
                params.insert(field.to_owned(), Json::Number(Number::from(limit)));
            }
        }
        let info = JsonObject::from_iter([("id".to_owned(), Json::Text(entry.id.clone()))]);
        let deployment = DeploymentSpec::new(
            input.model_group.clone(),
            params,
            info,
            JsonObject::new(),
            String::new,
        )
        .map_err(|error| OperationError::InvalidRequest {
            message: error.to_string(),
        })?;
        deployments.push(deployment);
    }
    Ok(deployments)
}

fn build_samples(input: &Input) -> ModelGroupSamples {
    let mut deployments = BTreeMap::new();
    for entry in &input.stored {
        deployments.insert(
            entry.id.clone(),
            LatencySamples {
                latency: entry.latency.iter().map(|sample| sample.0).collect(),
                time_to_first_token_seconds: entry
                    .time_to_first_token_seconds
                    .iter()
                    .map(|sample| sample.0)
                    .collect(),
                minutes: entry.minutes.clone(),
            },
        );
    }
    ModelGroupSamples { deployments }
}

fn minute_of(now_millis: u64) -> Result<String, OperationError> {
    let now = UNIX_EPOCH
        .checked_add(Duration::from_millis(now_millis))
        .ok_or_else(|| OperationError::InvalidRequest {
            message: format!("{now_millis} milliseconds is not an instant"),
        })?;
    Ok(minute_label(now))
}
