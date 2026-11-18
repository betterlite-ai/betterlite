use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_router::cooldown_decisions::failure::FailureClass;
use betterlite_router::latency_observations::LatencyObservations;
use betterlite_router::latency_observations::event::{
    Channel, FailureEvent, RequestChannels, SuccessEvent, TokenUsage,
};
use betterlite_router::latency_observations::samples::{LatencySamples, ModelGroupSamples};
use betterlite_router::latency_observations::settings::Settings;
use betterlite_router::latency_sample_math;
use betterlite_router::routing_value_store::Clock;
use rust_decimal::Decimal;

use super::{
    Channels, DeploymentView, FailureKind, Input, Output, SampleKind, ScenarioError, Seconds,
    Statistic, Step, StepKind, StepOutput,
};

pub(crate) fn run(input: &Input) -> Result<Output, ScenarioError> {
    let scenario = Scenario::new(input)?;
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        scenario.advance(step.now_millis);
        steps.push(scenario.apply(step)?);
    }
    Ok(Output { steps })
}

type SharedClock = Arc<Mutex<f64>>;

struct Scenario {
    settings: Settings,
    clock: SharedClock,
    observations: LatencyObservations,
}

impl Scenario {
    fn new(input: &Input) -> Result<Self, ScenarioError> {
        let clock: SharedClock = Arc::new(Mutex::new(0.0_f64));
        let handle = Arc::clone(&clock);
        let clock_fn: Clock =
            Arc::new(move || *handle.lock().unwrap_or_else(PoisonError::into_inner));
        let settings = Settings {
            ttl_seconds: input.settings.ttl.0,
            lowest_latency_buffer: input.settings.lowest_latency_buffer.0,
            max_latency_list_size: input.settings.max_latency_list_size,
            ttft_percentile: input
                .settings
                .ttft_percentile
                .map(|percentile| percentile.0),
        };
        let observations = LatencyObservations::new(settings, clock_fn)?;
        Ok(Self {
            settings,
            clock,
            observations,
        })
    }

    // the probe's frozen wall clock reads the same millisecond count as Python's clock, as float seconds
    fn advance(&self, now_millis: u64) {
        *self.clock.lock().unwrap_or_else(PoisonError::into_inner) =
            Duration::from_millis(now_millis).as_secs_f64();
    }

    fn apply(&self, step: &Step) -> Result<StepOutput, ScenarioError> {
        let mut output = StepOutput {
            kind: step.kind,
            deployments: None,
            score: None,
        };
        match step.kind {
            StepKind::Success => self.success(step)?,
            StepKind::Failure => self.failure(step)?,
            StepKind::View => {
                output.deployments = Some(views(self.observations.samples(model_group(step)?)?));
            }
            StepKind::Score => output.score = Some(self.score(step)?),
        }
        Ok(output)
    }

    fn success(&self, step: &Step) -> Result<(), ScenarioError> {
        let start_millis = step.start_millis;
        let end_millis = added_millis(start_millis, step.elapsed_millis)?;
        let completion_start_time = match step.first_token_millis {
            Some(first_token) => {
                let completed = added_millis(start_millis, first_token)?;
                Some(millis_seconds(completed)?)
            }
            None => None,
        };
        let event = SuccessEvent {
            call_type: step.call_type.as_deref(),
            channels: channels(step),
            deployment_id: step.deployment_id.as_deref(),
            stream: step.stream,
            start_time: millis_seconds(start_millis)?,
            end_time: millis_seconds(end_millis)?,
            completion_start_time,
            usage: step.usage.map(|usage| TokenUsage {
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            }),
        };
        self.observations.record_success(&event)?;
        Ok(())
    }

    fn failure(&self, step: &Step) -> Result<(), ScenarioError> {
        self.observations.record_failure(&failure_event(step))?;
        Ok(())
    }

    fn score(&self, step: &Step) -> Result<Seconds, ScenarioError> {
        let samples = self.observations.samples(model_group(step)?)?;
        let score = self.score_window(step, deployment(step, &samples)?)?;
        Ok(Seconds(score))
    }

    fn score_window(&self, step: &Step, deployment: &LatencySamples) -> Result<f64, ScenarioError> {
        let first_token = &deployment.time_to_first_token_seconds;
        // LiteLLM scores the latency window when no first-token sample exists, so the adapter refuses that step
        if step.sample == SampleKind::Ttft && first_token.is_empty() {
            return Err(ScenarioError::Missing {
                what: "a first-token window to score",
            });
        }
        match (step.statistic, step.sample) {
            (Statistic::Mean, SampleKind::Latency) => {
                Ok(latency_sample_math::average(&deployment.latency))
            }
            (Statistic::Mean, SampleKind::Ttft) => match self.settings.ttft_percentile {
                None => Ok(latency_sample_math::average(first_token)),
                Some(_) => Err(ScenarioError::Unsupported {
                    what: "LiteLLM only averages the first-token window while ttft_percentile is unset",
                }),
            },
            (Statistic::Percentile, SampleKind::Ttft) => {
                let percentile = self
                    .settings
                    .ttft_percentile
                    .ok_or(ScenarioError::Missing {
                        what: "a configured ttft_percentile for a percentile score",
                    })?;
                latency_sample_math::percentile(first_token, percentile).ok_or(
                    ScenarioError::Missing {
                        what: "a first-token window to take the percentile of",
                    },
                )
            }
            (Statistic::Percentile, SampleKind::Latency) => Err(ScenarioError::Unsupported {
                what: "LiteLLM takes no percentile of the latency window",
            }),
        }
    }
}

fn failure_event(step: &Step) -> FailureEvent<'_> {
    FailureEvent {
        call_type: step.call_type.as_deref(),
        channels: channels(step),
        deployment_id: step.deployment_id.as_deref(),
        failure: failure_class(step.failure),
    }
}

const fn failure_class(failure: FailureKind) -> FailureClass {
    match failure {
        FailureKind::Timeout => FailureClass::Timeout,
        FailureKind::Other => FailureClass::Unclassified,
    }
}

fn model_group(step: &Step) -> Result<&str, ScenarioError> {
    step.model_group.as_deref().ok_or(ScenarioError::Missing {
        what: "a model_group",
    })
}

fn deployment<'samples>(
    step: &Step,
    samples: &'samples ModelGroupSamples,
) -> Result<&'samples LatencySamples, ScenarioError> {
    let deployment_id = step
        .deployment_id
        .as_deref()
        .ok_or(ScenarioError::Missing {
            what: "a deployment_id",
        })?;
    samples
        .deployments
        .get(deployment_id)
        .ok_or(ScenarioError::Missing {
            what: "a recorded deployment to score",
        })
}

const fn added_millis(start_millis: u64, offset_millis: u64) -> Result<u64, ScenarioError> {
    match start_millis.checked_add(offset_millis) {
        Some(total) => Ok(total),
        None => Err(ScenarioError::TimeOverflow {
            millis: start_millis,
        }),
    }
}

fn channels(step: &Step) -> RequestChannels<'_> {
    let metadata = step.model_group.as_deref();
    match step.channels {
        Channels::None => RequestChannels::default(),
        Channels::Metadata => RequestChannels {
            litellm_metadata: Channel::Absent,
            metadata: Channel::Present(metadata),
        },
        Channels::LitellmMetadata => RequestChannels {
            litellm_metadata: Channel::Present(metadata),
            metadata: Channel::Present(step.metadata_group.as_deref()),
        },
    }
}

fn views(samples: ModelGroupSamples) -> BTreeMap<String, DeploymentView> {
    samples
        .deployments
        .into_iter()
        .map(|(deployment_id, deployment)| (deployment_id, deployment_view(deployment)))
        .collect()
}

fn deployment_view(samples: LatencySamples) -> DeploymentView {
    DeploymentView {
        latency: samples.latency.into_iter().map(Seconds).collect(),
        time_to_first_token_seconds: samples
            .time_to_first_token_seconds
            .into_iter()
            .map(Seconds)
            .collect(),
        minutes: samples.minutes,
    }
}

// CPython reads an integer millisecond count as the correctly rounded seconds value, which the decimal division gives
fn millis_seconds(millis: u64) -> Result<Decimal, ScenarioError> {
    Decimal::from(millis)
        .checked_div(Decimal::from(1_000_i64))
        .ok_or(ScenarioError::TimeOverflow { millis })
}
