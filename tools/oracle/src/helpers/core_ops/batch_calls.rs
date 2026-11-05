use betterlite_core::batch::{
    execution::{Batch, BatchLimits, Item, Mode},
    selection::{all_responses, first_ordered_model_result},
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use tokio::{sync::watch, task::JoinSet};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Messages,
    Models,
    Deployments,
    AllResponses,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    mode: Operation,
    #[serde(default)]
    results: Vec<Outcome>,
    #[serde(default)]
    completion_order: Vec<usize>,
    #[serde(default)]
    models: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Error)]
enum ProbeError {
    #[error("probe completion failed: {0}")]
    Completion(String),
    #[error("completion gate closed")]
    Gate(#[source] watch::error::RecvError),
    #[error("completion gate index overflow")]
    Overflow,
}

#[derive(Serialize)]
struct Selection {
    // The helper envelope treats top-level null as an absent value.
    selected: Option<String>,
}

pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(failure)?;
        runtime.block_on(run(input))
    }
}

fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "BatchError".to_owned(),
        message: error.to_string(),
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

fn schedule(input: &Input) -> Result<Vec<usize>, Box<crate::helpers::HelperError>> {
    if !matches!(input.mode, Operation::Messages) && input.models.len() != input.results.len() {
        return Err(failure("models and results must have equal lengths"));
    }
    let order = if input.completion_order.is_empty() {
        (0..input.results.len()).collect()
    } else {
        input.completion_order.clone()
    };
    let mut sorted = order.clone();
    sorted.sort_unstable();
    if sorted != (0..input.results.len()).collect::<Vec<_>>() || input.results.len() > 100 {
        return Err(failure("invalid completion_order or batch size"));
    }
    Ok(order)
}

fn prepare(input: &Input) -> Result<Batch<Outcome, usize>, Box<crate::helpers::HelperError>> {
    let order = schedule(input)?;
    let mut items = Vec::new();
    for (index, outcome) in input.results.iter().enumerate() {
        let rank = order
            .iter()
            .position(|entry| *entry == index)
            .ok_or_else(|| failure("missing completion gate"))?;
        items.push(Item {
            key: input
                .models
                .get(index)
                .cloned()
                .unwrap_or_else(|| index.to_string()),
            request: outcome.clone(),
            context: rank,
        });
    }
    Ok(Batch {
        items,
        mode: match input.mode {
            Operation::Deployments => Mode::Race,
            Operation::Messages | Operation::Models | Operation::AllResponses => Mode::Ordered,
        },
        limits: BatchLimits {
            max_items: 100,
            max_bytes: 0x0001_0000,
            concurrency: 100,
            deadline: Duration::from_secs(10),
        },
    })
}

async fn run(input: &Input) -> crate::helpers::HelperResult {
    let batch = prepare(input)?;
    let mut owners = JoinSet::new();
    let (turn, _) = watch::channel(0_usize);
    let receiver = batch
        .start(
            move |outcome, rank| {
                let progress = turn.clone();
                async move {
                    progress
                        .subscribe()
                        .wait_for(|current| *current == rank)
                        .await
                        .map_err(ProbeError::Gate)?;
                    progress.send_replace(rank.checked_add(1).ok_or(ProbeError::Overflow)?);
                    match outcome.error {
                        Some(error) => Err(ProbeError::Completion(error)),
                        None => Ok(outcome.value),
                    }
                }
            },
            &mut owners,
        )
        .map_err(failure)?;
    let report = receiver.await.map_err(failure)?.map_err(failure)?;
    while let Some(joined) = owners.join_next().await {
        joined.map_err(failure)?;
    }
    encode(input.mode, report.results)
}

fn encode(
    mode: Operation,
    results: Vec<Result<Option<String>, ProbeError>>,
) -> crate::helpers::HelperResult {
    match mode {
        Operation::Messages => {
            serde_json::to_value(results.into_iter().map(Result::ok).collect::<Vec<_>>())
                .map_err(failure)
        }
        Operation::Models | Operation::Deployments => serde_json::to_value(Selection {
            selected: first_ordered_model_result(results).map_err(failure)?,
        })
        .map_err(failure),
        Operation::AllResponses => serde_json::to_value(all_responses(results)).map_err(failure),
    }
}
