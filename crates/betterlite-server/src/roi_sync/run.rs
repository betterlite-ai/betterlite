use std::error::Error as StdError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use betterlite_roi::estimator::models::EstimatorModel;
use betterlite_roi::records::ROISyncPhase;
use betterlite_roi::settings::ROISettings;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::roi_cache::store::ReportRepository;
use crate::roi_estimator::CompletionCaller;
use crate::roi_github::client::GitHub;
use crate::roi_github::transport::Transport;
use crate::roi_spend::SpendReader;

use super::analysis::Analyze;
use super::capabilities::CapabilityLookup;
use super::coordinator::SyncCoordinator;
use super::error::RunError;
use super::heartbeat::Heartbeat;
use super::ports::{GitHubFactory, SyncClock};
use super::status::{StatusChange, StatusReporter};

pub(crate) struct RunTask<T: Transport, Reader, Repository> {
    pub(crate) settings: ROISettings,
    pub(crate) repository: Repository,
    pub(crate) spend_reader: Reader,
    pub(crate) complete: CompletionCaller,
    pub(crate) transport: Option<T>,
    pub(crate) models: Option<Vec<EstimatorModel>>,
    pub(crate) factory: GitHubFactory<T>,
    pub(crate) coordinator: Option<Arc<dyn SyncCoordinator>>,
    pub(crate) owner: String,
    pub(crate) status: StatusReporter,
    pub(crate) clock: SyncClock,
    pub(crate) capabilities: Arc<dyn CapabilityLookup>,
    pub(crate) token: CancellationToken,
    pub(crate) finished: Arc<AtomicBool>,
}

pub(crate) async fn run<T, Reader, Repository>(mut task: RunTask<T, Reader, Repository>)
where
    T: Transport,
    Reader: SpendReader + Send + Sync,
    Repository: ReportRepository + Send + Sync,
    Repository::Error: Send,
{
    let stop = CancellationToken::new();
    let mut monitor = JoinSet::new();
    spawn_heartbeat(&task, &stop, &mut monitor);
    let mut client: Option<GitHub<T>> = None;
    let outcome = run_analysis(&mut task, &mut client, &mut monitor, &stop).await;
    report_outcome(&task.status, &outcome);
    stop_monitor(&mut monitor, &stop).await;
    if let Some(github) = client.take() {
        github.close();
    }
    task.status.apply(StatusChange::Finished);
    notify_coordinator(&task).await;
    task.finished.store(true, Ordering::SeqCst);
}

async fn run_analysis<T, Reader, Repository>(
    task: &mut RunTask<T, Reader, Repository>,
    client: &mut Option<GitHub<T>>,
    monitor: &mut JoinSet<()>,
    stop: &CancellationToken,
) -> Result<(), RunError<Repository::Error>>
where
    T: Transport,
    Reader: SpendReader + Send + Sync,
    Repository: ReportRepository + Send + Sync,
{
    let transport = task.transport.take();
    let mut analysis = Analyze {
        settings: &task.settings,
        models: task.models.as_deref(),
        complete: &task.complete,
        spend_reader: &task.spend_reader,
        repository: &task.repository,
        status: &task.status,
        clock: &task.clock,
        capabilities: &task.capabilities,
        coordinator: task.coordinator.as_ref(),
        owner: &task.owner,
        factory: &task.factory,
        transport,
    };
    tokio::select! {
        result = analysis.run(client, monitor, stop) => result,
        () = task.token.cancelled() => Err(RunError::Cancelled),
    }
}

fn spawn_heartbeat<T, Reader, Repository>(
    task: &RunTask<T, Reader, Repository>,
    stop: &CancellationToken,
    monitor: &mut JoinSet<()>,
) where
    T: Transport,
{
    let Some(lease) = task.coordinator.as_ref() else {
        return;
    };
    let heartbeat = Heartbeat {
        coordinator: Arc::clone(lease),
        owner: task.owner.clone(),
        status: task.status.clone(),
        run: task.token.clone(),
        stop: stop.clone(),
    };
    monitor.spawn(heartbeat.run());
}

fn report_outcome<Store>(status: &StatusReporter, outcome: &Result<(), RunError<Store>>)
where
    Store: StdError + 'static,
{
    match outcome {
        Ok(()) => {}
        Err(RunError::Cancelled) => status.apply(StatusChange::Cancelled),
        Err(error) => status.apply(StatusChange::Failed {
            message: error.phase_error_message(),
        }),
    }
}

async fn notify_coordinator<T, Reader, Repository>(task: &RunTask<T, Reader, Repository>)
where
    T: Transport,
{
    let Some(lease) = task.coordinator.as_ref() else {
        return;
    };
    let final_status = task.status.snapshot();
    if final_status.phase != ROISyncPhase::Complete {
        lease.finish(&task.owner, &final_status, None).await;
    }
}

pub(crate) async fn stop_monitor(monitor: &mut JoinSet<()>, stop: &CancellationToken) {
    stop.cancel();
    while monitor.join_next().await.is_some() {}
}
