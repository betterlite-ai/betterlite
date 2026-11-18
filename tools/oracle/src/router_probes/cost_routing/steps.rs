use std::sync::atomic::Ordering;
use std::time::{Duration, UNIX_EPOCH};

use betterlite_router::cost_routing::error::CostRoutingError;
use betterlite_router::cost_routing::keys::minute_label;
use betterlite_router::cost_routing::selection::Selection;
use betterlite_router::cost_routing::service::SuccessEvent;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use betterlite_router::routing_value_store::StoreError;
use betterlite_tokenizer::count::error::CountError;
use thiserror::Error;

use super::{Context, Found, Observation, Step, StepKind};

// the adapter's own failures, each holding the error it came from until the probe boundary renders it
#[derive(Debug, Error)]
pub(super) enum ScenarioError {
    #[error("the in-memory value store is invalid")]
    Local {
        #[from]
        source: StoreError,
    },
    #[error("a deployment is invalid")]
    Deployment {
        #[from]
        source: InvalidDeployment,
    },
    #[error("the tokenizer is unavailable: {message}")]
    Tokenizer { message: String },
    #[error("the text could not be counted")]
    Count {
        #[from]
        source: CountError,
    },
    #[error("a deployment selection failed: {source}")]
    Selection {
        #[from]
        source: CostRoutingError,
    },
    #[error("the step needs {what}")]
    Missing { what: &'static str },
}

pub(super) async fn apply(
    index: usize,
    step: &Step,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    match step.kind {
        // log_success_event and async_log_success_event are one map update, so both kinds run the same primitive
        StepKind::Success | StepKind::SuccessSync => {
            let Some(deployment_id) = step.deployment_id.as_deref() else {
                return Err(ScenarioError::Missing {
                    what: "a deployment id",
                });
            };
            context
                .router
                .record_success(&SuccessEvent {
                    call_type: step.call_type.as_deref(),
                    model_group: Some(context.group),
                    deployment_id: Some(deployment_id),
                    total_tokens: step.total_tokens.unwrap_or_default(),
                })
                .await;
            Ok(Observation::Recorded { step: index })
        }
        StepKind::Select => select(index, step, context).await,
        StepKind::State => Ok(state(index, context)),
    }
}

async fn select(
    index: usize,
    step: &Step,
    context: &Context<'_>,
) -> Result<Observation, ScenarioError> {
    let input_tokens = match step.input.as_deref() {
        Some(input) => i64::try_from(context.text.count(input).await?).unwrap_or(i64::MAX),
        None => 0,
    };
    let request = Selection {
        group: context.group,
        deployments: context.deployments,
        input_tokens,
    };
    let selected = context.router.select(context.costs, &request)?;
    let deployment_id = selected.and_then(|position| {
        context
            .deployments
            .get(position)
            .and_then(|deployment| deployment.id())
            .map(str::to_owned)
    });
    Ok(Observation::Selected {
        step: index,
        deployment_id,
    })
}

fn state(index: usize, context: &Context<'_>) -> Observation {
    let now = UNIX_EPOCH
        .checked_add(Duration::from_millis(context.clock.load(Ordering::SeqCst)))
        .unwrap_or(UNIX_EPOCH);
    let minute = minute_label(now);
    let entries = context
        .router
        .entries(context.group, &minute)
        .into_iter()
        .map(|entry| Found {
            deployment_id: entry.deployment_id,
            tpm: entry.usage.tokens,
            rpm: entry.usage.requests,
        })
        .collect();
    Observation::State {
        step: index,
        entries,
    }
}
