use std::collections::BTreeMap;
use std::convert::Infallible;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rand::TryRng;
use serde_json::Number;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::latency_observations::samples::{LatencySamples, MinuteUsage, ModelGroupSamples};
use crate::latency_routing::service::{LatencyRouting, Ranking, RoutingArgs, Selection};
use crate::router_core::error::RoutingError;

// the minute every fixture selection reads and every stored window sits under unless a case says otherwise
pub(super) const MINUTE: &str = "2026-01-01-00-00";

// which of the three sources get_available_deployments reads a limit from
#[derive(Clone, Copy)]
pub(super) enum Source {
    Top,
    Params,
    Info,
}

// one limit as the YAML spells it: null, a whole number, or a value that is no number at all
#[derive(Clone, Copy)]
pub(super) enum LimitValue {
    Null,
    Value(i64),
    Text(&'static str),
}

impl LimitValue {
    fn json(self) -> Json {
        match self {
            Self::Null => Json::Null,
            Self::Value(value) => Json::Number(Number::from(value)),
            Self::Text(text) => Json::Text(text.to_owned()),
        }
    }
}

// one fixture deployment: where its limits sit and what the model group's map holds for it
#[derive(Clone, Copy)]
pub(super) struct Spec {
    pub(super) id: &'static str,
    pub(super) limits: &'static [(&'static str, Source, LimitValue)],
    pub(super) at: &'static str,
    pub(super) stored: bool,
    pub(super) latency: &'static [f64],
    pub(super) ttft: &'static [f64],
    pub(super) used: (i64, i64),
}

impl Spec {
    // a deployment the map has no entry for, so LiteLLM seeds it with a zero latency and no counters
    pub(super) const fn unseen(id: &'static str) -> Self {
        Self {
            id,
            limits: &[],
            at: MINUTE,
            stored: false,
            latency: &[],
            ttft: &[],
            used: (0, 0),
        }
    }

    // a deployment whose stored windows and counters the case names, under the minute it picks
    pub(super) const fn stored(
        at: &'static str,
        id: &'static str,
        latency: &'static [f64],
        ttft: &'static [f64],
        used: (i64, i64),
    ) -> Self {
        Self {
            id,
            limits: &[],
            at,
            stored: true,
            latency,
            ttft,
            used,
        }
    }

    pub(super) fn deployment(&self) -> DeploymentSpec {
        let mut extra = JsonObject::new();
        let mut params =
            JsonObject::from_iter([("model".to_owned(), Json::Text("openai/probe".to_owned()))]);
        let mut info = JsonObject::from_iter([("id".to_owned(), Json::Text(self.id.to_owned()))]);
        for (field, source, value) in self.limits {
            let target = match source {
                Source::Top => &mut extra,
                Source::Params => &mut params,
                Source::Info => &mut info,
            };
            target.insert((*field).to_owned(), value.json());
        }
        DeploymentSpec::new("gpt-4o".to_owned(), params, info, extra, String::new)
            .expect("the fixture deployment is valid")
    }
}

// what a selection of the case is expected to answer
#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Ranked {
        scores: &'static [f64],
        candidates: &'static [&'static str],
        valid: &'static [&'static str],
        selected: Option<&'static str>,
    },
    Rejected {
        field: &'static str,
    },
}

// one selection: the pool, the stored map, the request and the one random draw it takes
pub(super) struct Case {
    pub(super) specs: Vec<Spec>,
    pub(super) stream: bool,
    pub(super) input_tokens: i64,
    pub(super) buffer: f64,
    pub(super) percentile: Option<f64>,
    pub(super) draw: u64,
    pub(super) outcome: Outcome,
}

// a generator whose every draw is the same value, so a uniform pick is deterministic
#[derive(Clone, Copy)]
pub(super) struct FixedDraw(pub(super) u64);

impl TryRng for FixedDraw {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(u32::try_from(self.0).unwrap_or(u32::MAX))
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(self.0)
    }

    // a selection fills no bytes, so the buffer keeps what the caller left in it
    fn try_fill_bytes(&mut self, _bytes: &mut [u8]) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub(super) fn check(case: Case) {
    let (deployments, samples) = build(case.specs);
    let args = RoutingArgs {
        lowest_latency_buffer: case.buffer,
        ttft_percentile: case.percentile,
    };
    let router = LatencyRouting { args };
    let selection = Selection {
        deployments: &deployments,
        samples: &samples,
        stream: case.stream,
        input_tokens: case.input_tokens,
        minute: MINUTE,
    };
    let mut random = FixedDraw(case.draw);
    let ranked = router.rank(&selection);
    match case.outcome {
        Outcome::Ranked {
            scores,
            candidates,
            valid,
            selected,
        } => {
            let ranking = ranked.expect("the fixture limits are numbers");
            assert_scores(&ranking, scores);
            assert_eq!(ids(&deployments, &ranking.candidates), candidates);
            assert_eq!(ids(&deployments, &ranking.valid), valid);
            let picked = ranking
                .choose(&mut random)
                .and_then(|position| deployment_id(&deployments, position));
            assert_eq!(picked, selected);
        }
        Outcome::Rejected { field } => {
            let error = ranked.expect_err("the fixture limit is no number");
            assert!(
                matches!(&error, RoutingError::Metric { field: named } if *named == field),
                "the selection answered {error:?}"
            );
        }
    }
}

fn build(specs: Vec<Spec>) -> (Vec<DeploymentSpec>, ModelGroupSamples) {
    let mut deployments = Vec::new();
    let mut stored = BTreeMap::new();
    for spec in specs {
        deployments.push(spec.deployment());
        if spec.stored {
            stored.insert(
                spec.id.to_owned(),
                LatencySamples {
                    latency: spec.latency.to_vec(),
                    time_to_first_token_seconds: spec.ttft.to_vec(),
                    minutes: BTreeMap::from_iter([(
                        spec.at.to_owned(),
                        MinuteUsage {
                            tpm: spec.used.0,
                            rpm: spec.used.1,
                        },
                    )]),
                },
            );
        }
    }
    (
        deployments,
        ModelGroupSamples {
            deployments: stored,
        },
    )
}

fn assert_scores(ranking: &Ranking, expected: &[f64]) {
    let scores: Vec<u64> = ranking.scores.iter().map(|score| score.to_bits()).collect();
    let wanted: Vec<u64> = expected.iter().map(|score| score.to_bits()).collect();
    assert_eq!(scores, wanted);
}

fn ids<'deployments>(
    deployments: &'deployments [DeploymentSpec],
    positions: &[usize],
) -> Vec<&'deployments str> {
    let mut found = Vec::new();
    for position in positions {
        if let Some(id) = deployment_id(deployments, *position) {
            found.push(id);
        }
    }
    found
}

fn deployment_id(deployments: &[DeploymentSpec], position: usize) -> Option<&str> {
    deployments.get(position).and_then(DeploymentSpec::id)
}
