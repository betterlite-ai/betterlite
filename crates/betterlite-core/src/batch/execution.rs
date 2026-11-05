use futures_util::future::{Either, select};
use serde::Serialize;
use std::{collections::BTreeMap, future::Future, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{
    sync::oneshot,
    task::{JoinError, JoinSet},
    time::{Instant, timeout_at},
};

mod bounds;

#[derive(Debug, Clone, Copy)]
pub struct BatchLimits {
    pub max_items: usize,
    pub max_bytes: usize,
    pub concurrency: usize,
    pub deadline: Duration,
}

#[derive(Debug, Error)]
pub enum BatchError {
    #[error("batch has {actual} items; maximum is {maximum}")]
    TooManyItems { actual: usize, maximum: usize },
    #[error("batch exceeds {maximum} serialized bytes")]
    TooManyBytes { maximum: usize },
    #[error("cannot serialize batch request")]
    Serialize(#[source] serde_json::Error),
    #[error("batch concurrency must be greater than zero")]
    ZeroConcurrency,
    #[error("batch deadline elapsed")]
    Deadline,
    #[error("batch deadline is outside the clock range")]
    DeadlineRange,
    #[error("batch task failed")]
    Join(#[source] JoinError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Ordered,
    Race,
}

#[derive(Serialize)]
pub struct Item<Request, Context> {
    pub key: String,
    pub request: Request,
    #[serde(skip)]
    pub context: Context,
}

pub struct Batch<Request, Context> {
    pub items: Vec<Item<Request, Context>>,
    pub limits: BatchLimits,
    pub mode: Mode,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Duplicate {
    pub key: String,
    pub first: usize,
    pub second: usize,
}

#[derive(Debug)]
pub struct Report<Response, Failure> {
    pub results: Vec<Result<Option<Response>, Failure>>,
    pub duplicates: Vec<Duplicate>,
}

pub type Completion<Response, Failure> =
    oneshot::Receiver<Result<Report<Response, Failure>, BatchError>>;

impl<Request, Context> Batch<Request, Context>
where
    Request: Serialize + Send + 'static,
    Context: Send + 'static,
{
    pub fn start<Response, Failure, Execute, Call>(
        self,
        execute: Execute,
        owners: &mut JoinSet<()>,
    ) -> Result<Completion<Response, Failure>, BatchError>
    where
        Response: Send + 'static,
        Failure: Send + 'static,
        Execute: Fn(Request, Context) -> Call + Send + Sync + 'static,
        Call: Future<Output = Result<Option<Response>, Failure>> + Send + 'static,
    {
        bounds::validate(&self.items, self.limits)?;
        let expires = Instant::now()
            .checked_add(self.limits.deadline)
            .ok_or(BatchError::DeadlineRange)?;
        let duplicates = bounds::duplicates(&self.items);
        let (mut sender, receiver) = oneshot::channel();
        owners.spawn(async move {
            let mut tasks = JoinSet::new();
            let result = match select(
                Box::pin(sender.closed()),
                Box::pin(timeout_at(expires, run(self, execute, &mut tasks))),
            )
            .await
            {
                Either::Left(_) => None,
                Either::Right((outcome, _)) => Some(outcome.unwrap_or(Err(BatchError::Deadline))),
            };
            tasks.shutdown().await;
            if let Some(outcome) = result {
                let _sent = sender.send(outcome.map(|results| Report {
                    results,
                    duplicates,
                }));
            }
        });
        Ok(receiver)
    }
}

async fn run<Request, Context, Response, Failure, Execute, Call>(
    batch: Batch<Request, Context>,
    execute: Execute,
    tasks: &mut JoinSet<(usize, Result<Option<Response>, Failure>)>,
) -> Result<Vec<Result<Option<Response>, Failure>>, BatchError>
where
    Request: Send + 'static,
    Context: Send + 'static,
    Response: Send + 'static,
    Failure: Send + 'static,
    Execute: Fn(Request, Context) -> Call + Send + Sync + 'static,
    Call: Future<Output = Result<Option<Response>, Failure>> + Send + 'static,
{
    let executor = Arc::new(execute);
    let mut pending = batch.items.into_iter().enumerate();
    let mut ordered = BTreeMap::new();
    loop {
        while tasks.len() < batch.limits.concurrency {
            let Some((index, item)) = pending.next() else {
                break;
            };
            let worker = Arc::clone(&executor);
            tasks.spawn(async move { (index, worker(item.request, item.context).await) });
        }
        let Some(joined) = tasks.join_next().await else {
            break;
        };
        let (index, result) = joined.map_err(BatchError::Join)?;
        match (batch.mode, result) {
            (Mode::Race, success @ Ok(_)) => return Ok(vec![success]),
            (Mode::Race, Err(_)) => {}
            (Mode::Ordered, outcome) => {
                ordered.insert(index, outcome);
            }
        }
    }
    Ok(ordered.into_values().collect())
}
