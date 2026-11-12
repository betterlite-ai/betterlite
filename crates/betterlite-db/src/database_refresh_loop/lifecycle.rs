use std::error::Error;
use std::fmt::Display;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::future::{Either, select};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::task::{JoinError, JoinSet};
use tokio::time::sleep;

use crate::database_refresh::access::RefreshTrigger as _;
use crate::database_refresh::generation::{GenerationError, GenerationOptions, PoolGeneration};
use crate::database_refresh::handle::PoolProvider;
use crate::database_tokens::expiry::TokenAuthKind;

use super::clock::RefreshClock;
use super::endpoint::WriterEndpoint;
use super::minting::TokenMinter;
use super::port::{RefreshPort, RefreshPortConfig, RefreshPortError};
use super::schedule::{FALLBACK_REFRESH_SECONDS, TOKEN_REFRESH_BUFFER_SECONDS};

// float_arithmetic is denied; a fused multiply-add of the reciprocal gives seconds / 60
const MINUTE_FRACTION: f64 = 0.016_666_666_666_666_666;

pub struct RefreshLoopConfig<Minter> {
    pub writer_endpoint: WriterEndpoint,
    pub minter: Minter,
    pub clock: Arc<dyn RefreshClock>,
}

pub struct LoopTask {
    pub stop: Arc<Notify>,
    pub tasks: JoinSet<()>,
}

pub struct DatabaseRefreshLoop<P, M, C>
where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    pub generation: Arc<PoolGeneration<P, RefreshPort<P, M, C>, C>>,
    pub port: Arc<RefreshPort<P, M, C>>,
    pub task: Mutex<Option<LoopTask>>,
}

enum LoopStep {
    Continue,
    Stop,
}

fn report_error(prefix: &str, label: &str, error: &(dyn Display + Send + Sync)) {
    tracing::error!(
        "{prefix}Error in {label} refresh loop: {error}. Retrying in {FALLBACK_REFRESH_SECONDS}s..."
    );
}

async fn sleep_or_stop(wait: Duration, stop: &Notify) -> bool {
    let timer = Box::pin(sleep(wait));
    let stopping = Box::pin(stop.notified());
    matches!(select(timer, stopping).await, Either::Right(_))
}

async fn refresh_or_stop<P, M, C>(
    port: &RefreshPort<P, M, C>,
    stop: &Notify,
) -> Option<Result<(), RefreshPortError<P, M, C>>>
where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    let refreshed = Box::pin(port.refresh());
    let stopping = Box::pin(stop.notified());
    match select(refreshed, stopping).await {
        Either::Left((outcome, _stopping)) => Some(outcome),
        Either::Right(((), _refreshed)) => None,
    }
}

async fn error_sleep(
    prefix: &str,
    label: &str,
    error: &(dyn Display + Send + Sync),
    stop: &Notify,
) -> LoopStep {
    report_error(prefix, label, error);
    if sleep_or_stop(Duration::from_secs(FALLBACK_REFRESH_SECONDS), stop).await {
        LoopStep::Stop
    } else {
        LoopStep::Continue
    }
}

async fn iteration<P, M, C>(
    port: &RefreshPort<P, M, C>,
    prefix: &str,
    label: &'static str,
    stop: &Notify,
) -> LoopStep
where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    let schedule = match port.schedule() {
        Ok(schedule) => schedule,
        Err(error) => return error_sleep(prefix, label, &error, stop).await,
    };
    // the 30-second floor keeps the wait positive; the source still guards the sleep
    if schedule.wait > Duration::ZERO {
        tracing::info!(
            "{prefix}{label} refresh scheduled in {:.0} seconds ({:.1} minutes)",
            schedule.seconds,
            schedule.seconds.mul_add(MINUTE_FRACTION, 0.0_f64)
        );
        if sleep_or_stop(schedule.wait, stop).await {
            tracing::info!("{prefix}{label} refresh loop cancelled");
            return LoopStep::Stop;
        }
    }
    tracing::info!("{prefix}Proactively refreshing {label}...");
    match refresh_or_stop(port, stop).await {
        None => {
            tracing::info!("{prefix}{label} refresh loop cancelled");
            LoopStep::Stop
        }
        Some(Ok(())) => LoopStep::Continue,
        Some(Err(error)) => error_sleep(prefix, label, &error, stop).await,
    }
}

async fn refresh_loop<P, M, C>(
    port: Arc<RefreshPort<P, M, C>>,
    prefix: String,
    label: &'static str,
    stop: Arc<Notify>,
) where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    tracing::info!(
        "{prefix}{label} refresh loop started. Tokens will be refreshed {TOKEN_REFRESH_BUFFER_SECONDS}s before expiration."
    );
    loop {
        match iteration(port.as_ref(), &prefix, label, &stop).await {
            LoopStep::Continue => {}
            LoopStep::Stop => return,
        }
    }
}

impl<P, M, C> DatabaseRefreshLoop<P, M, C>
where
    P: PoolProvider + 'static,
    M: TokenMinter + 'static,
    C: Error + Send + Sync + 'static,
{
    pub fn new(
        provider: Arc<P>,
        options: GenerationOptions,
        initial: Option<Arc<P::Handle>>,
        config: RefreshLoopConfig<M>,
        on_replaced: Option<Arc<dyn Fn() -> Result<(), C> + Send + Sync>>,
    ) -> Result<Self, GenerationError> {
        // B713 resolves the same descriptor from these two options fields
        let auth_kind = options
            .token_auth
            .or_else(|| options.iam_token_db_auth.then_some(TokenAuthKind::Rds));
        let port = Arc::new(RefreshPort::new(RefreshPortConfig {
            auth_kind,
            writer_endpoint: config.writer_endpoint,
            iam_endpoint: options.iam_endpoint.clone(),
            url: options.url.clone(),
            clock: config.clock,
            minter: config.minter,
        }));
        let generation = Arc::new(PoolGeneration::new(
            provider,
            options,
            initial,
            Some(Arc::clone(&port)),
            on_replaced,
        )?);
        port.attach(&generation);
        Ok(Self {
            generation,
            port,
            task: Mutex::new(None),
        })
    }

    pub fn start(&self) {
        if !self.generation.iam_token_db_auth() {
            tracing::debug!("Database token auth not enabled, skipping token refresh task");
            return;
        }
        if Handle::try_current().is_err() {
            tracing::error!("cannot start the database token refresh task without a tokio runtime");
            return;
        }
        {
            let mut task = self.task.lock().unwrap_or_else(PoisonError::into_inner);
            if task.is_some() {
                tracing::debug!("Token refresh task already running");
                return;
            }
            let stop = Arc::new(Notify::new());
            let mut tasks = JoinSet::new();
            let prefix = self.generation.options.log_prefix.clone();
            let label = self.generation.token_label();
            tasks.spawn(refresh_loop(
                Arc::clone(&self.port),
                prefix.clone(),
                label,
                Arc::clone(&stop),
            ));
            *task = Some(LoopTask { stop, tasks });
            drop(task);
            tracing::info!("{prefix}Started {label} proactive refresh background task");
        }
    }

    #[must_use]
    pub fn has_task(&self) -> bool {
        self.task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    pub async fn stop(&self) -> Result<(), JoinError> {
        let taken = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(task) = taken else {
            return Ok(());
        };
        task.stop.notify_one();
        let mut tasks = task.tasks;
        while let Some(outcome) = tasks.join_next().await {
            if let Err(error) = outcome
                && !error.is_cancelled()
            {
                return Err(error);
            }
        }
        let prefix = self.generation.options.log_prefix.as_str();
        let label = self.generation.token_label();
        tracing::info!("{prefix}Stopped {label} refresh background task");
        Ok(())
    }
}
