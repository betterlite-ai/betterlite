use std::error::Error as StdError;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::rag::ingestion::store::{RagStore, StoreRequest, StoreResult};
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep};

use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_ingestion::error::BedrockIngestionError;
use crate::bedrock_provisioning::calls::agent;
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::provision::BedrockProvisioner;
use crate::bedrock_provisioning::provision::error::ProvisionError;

const INGESTION_POLL_INTERVAL: Duration = Duration::from_secs(2);
const COMPLETE: &str = "COMPLETE";
const FAILED: &str = "FAILED";

#[derive(Debug, thiserror::Error)]
pub enum IngestError<TransportFailure>
where
    TransportFailure: StdError,
{
    #[error(transparent)]
    Provision(#[from] ProvisionError<TransportFailure>),
    #[error(transparent)]
    Call(#[from] CallError),
    #[error(transparent)]
    Ingestion(#[from] BedrockIngestionError<TransportFailure>),
    #[error("an S3 bucket is required to store a file and none is known yet")]
    MissingBucket,
    #[error("a data source id is required to start an ingestion job and none is known yet")]
    MissingDataSource,
    #[error("a knowledge base id is required to start an ingestion job and none is known yet")]
    MissingKnowledgeBase,
}

#[derive(Debug)]
pub struct BedrockIngest<'deps, Transport> {
    pub provisioner: Mutex<BedrockProvisioner<'deps, Transport>>,
}

#[async_trait]
impl<Transport> RagStore for BedrockIngest<'_, Transport>
where
    Transport: AwsRagTransport,
{
    type Error = IngestError<Transport::Error>;

    async fn store(&self, request: StoreRequest<'_>) -> Result<StoreResult, Self::Error> {
        let mut provisioner = self.provisioner.lock().await;
        provisioner.store(request).await
    }
}

pub(super) async fn wait_for_ingestion<Transport>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    region: &str,
    knowledge_base: &str,
    data_source: &str,
    job_id: &str,
) -> Result<(), IngestError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let start = Instant::now();
    let seconds = u64::try_from(provisioner.clients.settings.ingestion_timeout).unwrap_or_default();
    let limit = Duration::from_secs(seconds);
    while start.elapsed() < limit {
        let call = agent::ingestion_job_call(
            region,
            knowledge_base,
            data_source,
            agent::IngestionJobCall::Get(job_id),
        )?;
        let response = provisioner
            .send(&call)
            .await?
            .require_success(call.operation)?;
        let job = agent::ingestion_job(&response.body, agent::GET_INGESTION_JOB)?;
        if !poll_again(&job, job_id) {
            return Ok(());
        }
        sleep(INGESTION_POLL_INTERVAL).await;
    }
    Ok(())
}

// whether the job is still running; a completed, failed or unknown status stops the wait
fn poll_again(job: &agent::IngestionJob, job_id: &str) -> bool {
    match job.status.as_deref() {
        Some(COMPLETE) => {
            let documents = job
                .statistics
                .as_ref()
                .and_then(|statistics| statistics.new_documents_indexed)
                .unwrap_or_default();
            tracing::info!(
                job = job_id,
                documents,
                "the Bedrock ingestion job completed"
            );
            false
        }
        Some(FAILED) => {
            tracing::error!(
                job = job_id,
                reasons = ?job.failure_reasons,
                "the Bedrock ingestion job failed"
            );
            false
        }
        Some("STARTING" | "IN_PROGRESS") => true,
        other => {
            tracing::warn!(job = job_id, status = ?other, "an unknown ingestion status");
            false
        }
    }
}

mod tests;
