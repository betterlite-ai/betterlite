use std::error::Error as StdError;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::rag::ingestion::store::{RagStore, StoreRequest, StoreResult};
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep};

use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::calls::{agent, s3};
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

impl<Transport> BedrockProvisioner<'_, Transport>
where
    Transport: AwsRagTransport,
{
    // Bedrock embeds the file inside the ingestion job, so there is no vector to hand back
    pub fn embed(&self, _chunks: &[String]) -> Option<Vec<Vec<f64>>> {
        None
    }

    pub async fn store(
        &mut self,
        request: StoreRequest<'_>,
    ) -> Result<StoreResult, IngestError<Transport::Error>> {
        self.ensure_config_initialized().await?;
        let knowledge_base_id = self.clients.settings.knowledge_base_id.clone();
        let (Some(content), Some(filename)) = (request.file_content, request.filename) else {
            return Ok((knowledge_base_id, None));
        };
        if content.is_empty() || filename.is_empty() {
            return Ok((knowledge_base_id, None));
        }
        let bucket = self
            .state
            .s3_bucket
            .clone()
            .ok_or(IngestError::MissingBucket)?;
        let data_source = self
            .state
            .data_source_id
            .clone()
            .ok_or(IngestError::MissingDataSource)?;
        let knowledge_base = knowledge_base_id
            .clone()
            .ok_or(IngestError::MissingKnowledgeBase)?;
        let prefix = self.state.s3_prefix.trim_end_matches('/');
        let key = format!("{prefix}/{filename}");
        let region = self.clients.settings.aws_region_name.clone();
        let call = s3::put_object(&region, &bucket, &key, request.content_type, content);
        self.send(&call).await?.require_success(call.operation)?;
        let call = agent::ingestion_job_call(
            &region,
            &knowledge_base,
            &data_source,
            agent::IngestionJobCall::Start,
        )?;
        let response = self.send(&call).await?.require_success(call.operation)?;
        let job_id = agent::ingestion_job_id(&response.body, agent::START_INGESTION_JOB)?;
        if self.clients.settings.wait_for_ingestion {
            self.wait_for_ingestion(&region, &knowledge_base, &data_source, &job_id)
                .await?;
        }
        Ok((Some(knowledge_base), Some(key)))
    }

    async fn wait_for_ingestion(
        &self,
        region: &str,
        knowledge_base: &str,
        data_source: &str,
        job_id: &str,
    ) -> Result<(), IngestError<Transport::Error>> {
        let start = Instant::now();
        let seconds = u64::try_from(self.clients.settings.ingestion_timeout).unwrap_or_default();
        let limit = Duration::from_secs(seconds);
        while start.elapsed() < limit {
            let call = agent::ingestion_job_call(
                region,
                knowledge_base,
                data_source,
                agent::IngestionJobCall::Get(job_id),
            )?;
            let response = self.send(&call).await?.require_success(call.operation)?;
            let job = agent::ingestion_job(&response.body, agent::GET_INGESTION_JOB)?;
            match job.status.as_deref() {
                Some(COMPLETE) => {
                    let documents = job
                        .statistics
                        .and_then(|statistics| statistics.new_documents_indexed)
                        .unwrap_or_default();
                    tracing::info!(
                        job = job_id,
                        documents,
                        "the Bedrock ingestion job completed"
                    );
                    return Ok(());
                }
                Some(FAILED) => {
                    tracing::error!(
                        job = job_id,
                        reasons = ?job.failure_reasons,
                        "the Bedrock ingestion job failed"
                    );
                    return Ok(());
                }
                Some("STARTING" | "IN_PROGRESS") => sleep(INGESTION_POLL_INTERVAL).await,
                other => {
                    tracing::warn!(job = job_id, status = ?other, "an unknown ingestion status");
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

mod tests;
