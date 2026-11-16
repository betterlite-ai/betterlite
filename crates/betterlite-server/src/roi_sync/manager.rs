use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_db::search_tools::convert::isoformat;
use betterlite_roi::estimator::models::EstimatorModel;
use betterlite_roi::records::{ROISyncPhase, ROISyncStatus};
use betterlite_roi::settings::ROISettings;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::roi_cache::store::ReportRepository;
use crate::roi_estimator::CompletionCaller;
use crate::roi_github::transport::Transport;
use crate::roi_spend::SpendReader;

use super::capabilities::CapabilityLookup;
use super::coordinator::SyncCoordinator;
use super::ports::{GitHubFactory, SyncClock, UuidSource};
use super::run::{RunTask, run as run_task};
use super::status::{IDLE_STAGE, SPEND_STAGE, StatusChange, StatusReporter, timed_status};

pub struct SyncRun<T: Transport, Reader, Repository> {
    pub settings: ROISettings,
    pub repository: Repository,
    pub spend_reader: Reader,
    pub complete: CompletionCaller,
    pub github_transport: Option<T>,
    pub estimator_models: Option<Vec<EstimatorModel>>,
    pub coordinator: Option<Arc<dyn SyncCoordinator>>,
    pub scheduled_interval: f64,
}

struct RunSlot {
    tasks: JoinSet<()>,
    cancel: CancellationToken,
    finished: Arc<AtomicBool>,
    coordinator: Option<Arc<dyn SyncCoordinator>>,
    owner: String,
}

pub struct SyncManager<T: Transport> {
    factory: GitHubFactory<T>,
    reporter: StatusReporter,
    slot: Mutex<Option<RunSlot>>,
    start_lock: AsyncMutex<()>,
    next_id: UuidSource,
    capabilities: Arc<dyn CapabilityLookup>,
}

impl<T: Transport + 'static> SyncManager<T> {
    #[must_use]
    pub fn new(
        factory: GitHubFactory<T>,
        clock: SyncClock,
        next_id: UuidSource,
        capabilities: Arc<dyn CapabilityLookup>,
    ) -> Self {
        let reporter = StatusReporter {
            stored: Arc::new(Mutex::new(ROISyncStatus {
                running: false,
                phase: ROISyncPhase::Idle,
                stage: IDLE_STAGE.to_owned(),
                done: 0,
                total: 0,
                estimated: 0,
                reused: 0,
                needs_attention: 0,
                error: None,
                started_at: None,
                finished_at: None,
                next_update: None,
                elapsed_seconds: 0,
                remaining_seconds: None,
            })),
            clock,
        };
        Self {
            factory,
            reporter,
            slot: Mutex::new(None),
            start_lock: AsyncMutex::new(()),
            next_id,
            capabilities,
        }
    }

    #[must_use]
    pub fn status(&self) -> ROISyncStatus {
        let now = (self.reporter.clock)();
        let stored = self
            .reporter
            .stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        timed_status(stored, now)
    }

    #[must_use]
    pub async fn start<Reader, Repository>(&self, run: SyncRun<T, Reader, Repository>) -> bool
    where
        Reader: SpendReader + Send + Sync + 'static,
        Repository: ReportRepository + Send + Sync + 'static,
        Repository::Error: Send,
    {
        let _guard = self.start_lock.lock().await;
        if run.settings.repos.is_empty() || run.settings.estimator_model.is_empty() {
            return false;
        }
        if !self.clear_previous(run.coordinator.as_ref()).await {
            return false;
        }
        let owner = (self.next_id)().to_string();
        let initial = initial_status(isoformat((self.reporter.clock)()));
        let lease = run.coordinator.as_ref();
        let acquired = acquire_lease(lease, &owner, &initial, run.scheduled_interval).await;
        if !acquired {
            return false;
        }
        {
            let mut stored = self
                .reporter
                .stored
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *stored = initial;
        }
        self.launch(run, owner).await;
        true
    }

    #[must_use]
    pub async fn cancel(&self) -> bool {
        let _guard = self.start_lock.lock().await;
        self.cancel_run().await
    }

    async fn clear_previous(&self, coordinator: Option<&Arc<dyn SyncCoordinator>>) -> bool {
        let running = self
            .reporter
            .stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .running;
        if !running {
            return true;
        }
        let Some(lease) = coordinator else {
            return false;
        };
        let shared = lease.status().await;
        if shared.is_some_and(|status| status.running) {
            return false;
        }
        self.cancel_run().await;
        true
    }

    async fn cancel_run(&self) -> bool {
        let slot = self
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let Some(mut slot) = slot else {
            return false;
        };
        if slot.finished.load(Ordering::SeqCst) {
            return false;
        }
        slot.cancel.cancel();
        while slot.tasks.join_next().await.is_some() {}
        self.reporter.apply(StatusChange::Cancelled);
        self.reporter.apply(StatusChange::FinishedComputed);
        if let Some(lease) = slot.coordinator.as_ref() {
            let final_status = self.reporter.snapshot();
            lease.finish(&slot.owner, &final_status, None).await;
        }
        true
    }

    async fn launch<Reader, Repository>(&self, run: SyncRun<T, Reader, Repository>, owner: String)
    where
        Reader: SpendReader + Send + Sync + 'static,
        Repository: ReportRepository + Send + Sync + 'static,
        Repository::Error: Send,
    {
        let cancel = CancellationToken::new();
        let finished = Arc::new(AtomicBool::new(false));
        let coordinator = run.coordinator;
        let task = RunTask {
            settings: run.settings,
            repository: run.repository,
            spend_reader: run.spend_reader,
            complete: run.complete,
            transport: run.github_transport,
            models: run.estimator_models,
            factory: Arc::clone(&self.factory),
            coordinator: coordinator.clone(),
            owner: owner.clone(),
            status: self.reporter.clone(),
            clock: Arc::clone(&self.reporter.clock),
            capabilities: Arc::clone(&self.capabilities),
            token: cancel.clone(),
            finished: Arc::clone(&finished),
        };
        let mut tasks = JoinSet::new();
        tasks.spawn(run_task(task));
        let slot = RunSlot {
            tasks,
            cancel,
            finished,
            coordinator,
            owner,
        };
        self.replace_slot(slot).await;
    }

    async fn replace_slot(&self, slot: RunSlot) {
        let retired = self
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(slot);
        if let Some(mut previous) = retired {
            while previous.tasks.join_next().await.is_some() {}
        }
    }
}

async fn acquire_lease(
    coordinator: Option<&Arc<dyn SyncCoordinator>>,
    owner: &str,
    initial: &ROISyncStatus,
    scheduled_interval: f64,
) -> bool {
    match coordinator {
        Some(lease) => lease.acquire(owner, initial, scheduled_interval).await,
        None => true,
    }
}

fn initial_status(started_at: String) -> ROISyncStatus {
    ROISyncStatus {
        running: true,
        phase: ROISyncPhase::Spend,
        stage: SPEND_STAGE.to_owned(),
        done: 0,
        total: 0,
        estimated: 0,
        reused: 0,
        needs_attention: 0,
        error: None,
        started_at: Some(started_at),
        finished_at: None,
        next_update: None,
        elapsed_seconds: 0,
        remaining_seconds: None,
    }
}
