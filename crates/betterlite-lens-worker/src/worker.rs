use std::sync::Arc;
use std::time::Duration;

use betterlite_lens::analysis::{AnalysisError, ModelCall, ReadContent, ReportProgress};
use betterlite_lens::coordinator::{self, CoordinatorError};
use betterlite_lens::extraction::ExtractionError;
use betterlite_lens::models::execution::Coverage;
use betterlite_lens::models::messages::{Claim, JobResult, ModelRequest, ModelResult, Progress};
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::api::{LensApi, WorkerError};
use crate::failure::{analysis_failure, failure_message};

pub const RETRY_STATUSES: [u16; 4] = [429, 502, 503, 504];
pub const MAX_MODEL_RETRIES: u32 = 2;
pub const HEARTBEAT_EVERY: Duration = Duration::from_secs(30);
pub const IDLE_PAUSE: Duration = Duration::from_secs(10);

pub struct LensWorker<A: LensApi> {
    pub api: Arc<A>,
}

impl<A: LensApi> Clone for LensWorker<A> {
    fn clone(&self) -> Self {
        Self {
            api: Arc::clone(&self.api),
        }
    }
}

impl<A: LensApi> LensWorker<A> {
    pub async fn model_request(
        &self,
        prefix: &str,
        body: ModelRequest,
    ) -> Result<ModelResult, WorkerError> {
        let mut attempt: u32 = 0;
        loop {
            let error = match self.api.model(prefix, body.clone()).await {
                Ok(result) => return Ok(result),
                Err(error) => error,
            };
            if attempt >= MAX_MODEL_RETRIES || !retryable(&error) {
                return Err(error);
            }
            sleep(Duration::from_secs(1_u64 << attempt)).await;
            attempt = attempt.saturating_add(1);
        }
    }

    pub async fn run_once(&self) -> Result<bool, WorkerError> {
        let Some(claim) = self.api.claim().await? else {
            return Ok(false);
        };
        let prefix = format!("/lens/worker/{}/{}", claim.lens_id, claim.job.id);
        let job_id = claim.job.id.clone();
        let mut pulse = self.start_heartbeat(&prefix);
        let status = self.drive(claim, &prefix, &job_id).await;
        pulse.abort_all();
        while pulse.join_next().await.is_some() {}
        status
    }

    pub async fn run_forever(&self) -> ! {
        loop {
            if let Err(error) = self.run_once().await {
                tracing::warn!("Worker could not reach Lens ({})", error.kind());
            }
            sleep(IDLE_PAUSE).await;
        }
    }

    fn start_heartbeat(&self, prefix: &str) -> JoinSet<()> {
        let mut pulses = JoinSet::new();
        let api = Arc::clone(&self.api);
        let path = prefix.to_owned();
        pulses.spawn(async move { while beat(api.as_ref(), &path).await {} });
        pulses
    }

    async fn drive(&self, claim: Claim, prefix: &str, job_id: &str) -> Result<bool, WorkerError> {
        let interrupted = match self.analyze(claim, prefix).await {
            Ok(result) => self.save(prefix, &result).await.err(),
            Err(error) => Some(error),
        };
        let Some(error) = interrupted else {
            return Ok(true);
        };
        tracing::warn!("Analysis {job_id} interrupted ({})", error.kind());
        let failed = JobResult {
            assessments: Vec::new(),
            findings: Vec::new(),
            coverage: Coverage::default(),
            error: error.message(),
        };
        if let Err(answer_error) = self.api.result(prefix, &failed).await
            && !matches!(answer_error, WorkerError::Status { status: 409, .. })
        {
            return Err(answer_error);
        }
        Ok(true)
    }

    async fn save(&self, prefix: &str, result: &JobResult) -> Result<(), Failure> {
        let saved = self
            .api
            .result(prefix, result)
            .await
            .map_err(Failure::Api)?;
        tracing::debug!(status = saved.status, "the analysis result was saved");
        Ok(())
    }

    async fn analyze(&self, claim: Claim, prefix: &str) -> Result<JobResult, Failure> {
        let sample = self.api.sample(prefix).await.map_err(Failure::Api)?;
        let (read, progress) = self.api_ports(prefix);
        let model = self.model_port(prefix);
        coordinator::analyze_sample(claim, sample, read, model, progress)
            .await
            .map_err(Failure::Analysis)
    }

    fn api_ports(&self, prefix: &str) -> (ReadContent, ReportProgress) {
        let read_api = Arc::clone(&self.api);
        let read_path = prefix.to_owned();
        let read: ReadContent = Arc::new(move |execution_id, cursor, offset| {
            let api = Arc::clone(&read_api);
            let path = read_path.clone();
            Box::pin(async move {
                api.content(&path, &execution_id, &cursor, offset)
                    .await
                    .map_err(|error| AnalysisError::Callback(failure_message(&error)))
            })
        });
        let progress_api = Arc::clone(&self.api);
        let progress_path = prefix.to_owned();
        let progress: ReportProgress = Arc::new(move |stage, coverage| {
            let api = Arc::clone(&progress_api);
            let path = progress_path.clone();
            Box::pin(async move {
                api.progress(&path, Progress { stage, coverage })
                    .await
                    .map_err(|error| AnalysisError::Callback(failure_message(&error)))
            })
        });
        (read, progress)
    }

    fn model_port(&self, prefix: &str) -> ModelCall {
        let port_worker = self.clone();
        let port_path = prefix.to_owned();
        Arc::new(move |body| {
            let worker = port_worker.clone();
            let path = port_path.clone();
            Box::pin(async move {
                worker
                    .model_request(&path, body)
                    .await
                    .map_err(|error| AnalysisError::Callback(failure_message(&error)))
            })
        })
    }
}

async fn beat<A>(api: &A, prefix: &str) -> bool
where
    A: LensApi,
{
    sleep(HEARTBEAT_EVERY).await;
    api.heartbeat(prefix).await.is_ok()
}

fn retryable(error: &WorkerError) -> bool {
    match error {
        WorkerError::Timeout | WorkerError::Transport => true,
        WorkerError::Status { status, .. } => RETRY_STATUSES.contains(status),
        WorkerError::Decode(_text) | WorkerError::Storage(_text) => false,
    }
}

enum Failure {
    Api(WorkerError),
    Analysis(CoordinatorError),
}

impl Failure {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Api(error) => error.kind(),
            Self::Analysis(error) => coordinator_kind(error),
        }
    }

    fn message(&self) -> String {
        match self {
            Self::Api(error) => failure_message(error),
            Self::Analysis(error) => analysis_failure(error),
        }
    }
}

const fn coordinator_kind(error: &CoordinatorError) -> &'static str {
    match error {
        CoordinatorError::Analysis(inner)
        | CoordinatorError::Extraction(ExtractionError::Analysis(inner)) => match inner {
            AnalysisError::Decode(_) => "Decode",
            AnalysisError::Join(_) => "Join",
            AnalysisError::Json(_) => "Json",
            AnalysisError::Validation(_) => "Validation",
            AnalysisError::Store(_) => "Store",
            AnalysisError::Callback(_) => "Callback",
        },
        CoordinatorError::Extraction(ExtractionError::Count(_)) | CoordinatorError::Count(_) => {
            "Count"
        }
    }
}

#[path = "worker/tests/fixtures.rs"]
mod fixtures;
#[path = "worker/retry/tests.rs"]
mod retry;
#[path = "worker/run/tests.rs"]
mod run;
