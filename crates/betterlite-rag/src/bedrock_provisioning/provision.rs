use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::time::Duration;

use betterlite_context::aws_partition::get_aws_partition;
use betterlite_core::rag::ingestion::store::{StoreRequest, StoreResult};
use tokio::time::sleep;

use crate::bedrock_ingestion::client::{
    AwsRagResponse, AwsRagTransport, BedrockRagClients, SignedRequest,
};
use crate::bedrock_ingestion::error::BedrockIngestionError;
use crate::bedrock_ingestion::state::BedrockRagState;
use crate::bedrock_provisioning::calls::CallRequest;
use crate::bedrock_provisioning::calls::aoss;
use crate::bedrock_provisioning::calls::aoss::CollectionSelection;
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::calls::query::{self, CallerIdentity};
use crate::bedrock_provisioning::calls::{agent, s3};
use crate::bedrock_provisioning::ingest::{IngestError, wait_for_ingestion};
use crate::bedrock_provisioning::plan;
use crate::bedrock_provisioning::plan::RoleResources;
use crate::bedrock_provisioning::provision::error::ProvisionError;

pub mod error;
mod iam;
mod index;
mod resources;
pub(crate) mod tests;

const ROLE_PROPAGATION: Duration = Duration::from_secs(10);

pub struct BedrockProvisioner<'deps, Transport> {
    pub clients: BedrockRagClients<'deps, Transport>,
    pub state: BedrockRagState,
    pub ingest_name: Option<String>,
    pub unique_id: String,
    pub initialized: bool,
}

impl<Transport> Debug for BedrockProvisioner<'_, Transport> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("BedrockProvisioner")
            .field("state", &self.state)
            .field("ingest_name", &self.ingest_name)
            .field("unique_id", &self.unique_id)
            .field("initialized", &self.initialized)
            .finish_non_exhaustive()
    }
}

impl<Transport> BedrockProvisioner<'_, Transport>
where
    Transport: AwsRagTransport,
{
    // ast-grep-ignore: no-one-line-functions - BedrockRAGIngestion.embed returns None, and the ingestion job embeds the file
    pub const fn embed(&self, _chunks: &[String]) -> Option<Vec<Vec<f64>>> {
        None
    }

    pub async fn ensure_config_initialized(
        &mut self,
    ) -> Result<(), ProvisionError<Transport::Error>> {
        if self.initialized {
            return Ok(());
        }
        if self.clients.settings.knowledge_base_id.is_some() {
            self.state.auto_detect_config(&self.clients).await?;
        } else {
            self.create_infrastructure().await?;
        }
        self.initialized = true;
        Ok(())
    }

    pub async fn create_infrastructure(&mut self) -> Result<(), ProvisionError<Transport::Error>> {
        let unique_id = self.unique_id.clone();
        let kb_name = self
            .ingest_name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("litellm-kb-{unique_id}"));
        let identity = self.caller_identity().await?;
        let bucket = match self.clients.settings.s3_bucket.clone() {
            Some(configured) => configured,
            None => {
                self.state
                    .create_s3_bucket(&self.clients, &unique_id)
                    .await?
            }
        };
        self.state.s3_bucket = Some(bucket);
        let (collection_name, collection_arn) = self
            .create_opensearch_collection(&unique_id, &identity.account, &identity.arn)
            .await?;
        self.create_opensearch_index(&collection_name).await?;
        let role_arn = self
            .create_bedrock_role(&unique_id, &identity.account, &collection_arn)
            .await?;
        let knowledge_base_id = self
            .state
            .create_knowledge_base(&self.clients, &kb_name, &role_arn, &collection_arn)
            .await?;
        let data_source_id = self
            .state
            .create_data_source(&self.clients, &kb_name)
            .await?;
        self.state.data_source_id = Some(data_source_id);
        self.clients.settings.knowledge_base_id = Some(knowledge_base_id);
        Ok(())
    }

    pub async fn create_opensearch_collection(
        &mut self,
        unique_id: &str,
        account_id: &str,
        caller_arn: &str,
    ) -> Result<(String, String), ProvisionError<Transport::Error>> {
        let name = format!("litellm-kb-{unique_id}");
        resources::create_policies(self, &name, account_id, caller_arn).await?;
        let arn = resources::create_collection_and_wait(self, &name).await?;
        Ok((name, arn))
    }

    pub async fn create_opensearch_index(
        &self,
        collection_name: &str,
    ) -> Result<(), ProvisionError<Transport::Error>> {
        let region = self.clients.settings.aws_region_name.clone();
        let call = aoss::batch_get_collection(&region, CollectionSelection::Name(collection_name))?;
        let response = self.send(&call).await?.require_success(call.operation)?;
        let detail = aoss::first_collection(&response.body, aoss::BATCH_GET_COLLECTION)?;
        let endpoint = detail.collection_endpoint.ok_or(CallError::MissingEntry {
            operation: aoss::BATCH_GET_COLLECTION,
            field: "collectionEndpoint",
        })?;
        let url = index::index_url(&endpoint, &region)?;
        let body = serde_json::to_vec(&plan::index::index_body()).map_err(|source| {
            CallError::Serialize {
                operation: index::INDEX_OPERATION,
                source,
            }
        })?;
        index::create_index(self, &region, &url, body).await
    }

    pub async fn create_bedrock_role(
        &mut self,
        unique_id: &str,
        account_id: &str,
        collection_arn: &str,
    ) -> Result<String, ProvisionError<Transport::Error>> {
        let role_name = format!("litellm-bedrock-kb-{unique_id}");
        let region = self.clients.settings.aws_region_name.clone();
        let embedding_model = self.clients.settings.embedding_model.clone();
        // create_infrastructure sets the state bucket first; a configured one is the same name
        let bucket = self
            .state
            .s3_bucket
            .clone()
            .or_else(|| self.clients.settings.s3_bucket.clone())
            .ok_or(BedrockIngestionError::MissingS3Bucket)?;
        let role_resources = RoleResources {
            arn_prefix: get_aws_partition(Some(region.as_str())).arn_prefix,
            region: &region,
            account_id,
            embedding_model: &embedding_model,
            collection_arn,
            bucket: &bucket,
        };
        let role_arn = iam::create_role_with_policy(self, &role_resources, &role_name).await?;
        sleep(ROLE_PROPAGATION).await;
        Ok(role_arn)
    }

    pub async fn store(
        &mut self,
        request: StoreRequest<'_>,
    ) -> Result<StoreResult, IngestError<Transport::Error>> {
        self.ensure_config_initialized().await?;
        let configured_kb = self.clients.settings.knowledge_base_id.clone();
        let Some((content, filename)) = stored_file(request.file_content, request.filename) else {
            tracing::warn!("No file content or filename provided for Bedrock ingestion");
            return Ok((configured_kb, None));
        };
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
        let knowledge_base = configured_kb
            .clone()
            .ok_or(IngestError::MissingKnowledgeBase)?;
        let prefix = self.state.s3_prefix.trim_end_matches('/');
        let key = format!("{prefix}/{filename}");
        let region = self.clients.settings.aws_region_name.clone();
        let put = s3::put_object(&region, &bucket, &key, request.content_type, content);
        self.send(&put).await?.require_success(put.operation)?;
        let call = agent::ingestion_job_call(
            &region,
            &knowledge_base,
            &data_source,
            agent::IngestionJobCall::Start,
        )?;
        let response = self.send(&call).await?.require_success(call.operation)?;
        let job_id = agent::ingestion_job_id(&response.body, agent::START_INGESTION_JOB)?;
        if self.clients.settings.wait_for_ingestion {
            wait_for_ingestion(self, &region, &knowledge_base, &data_source, &job_id).await?;
        }
        Ok((Some(knowledge_base), Some(key)))
    }

    pub(crate) async fn send(
        &self,
        call: &CallRequest,
    ) -> Result<AwsRagResponse, ProvisionError<Transport::Error>> {
        let request = SignedRequest {
            service: call.service,
            signing_region: &call.signing_region,
            method: call.method,
            url: &call.url,
            headers: &call.headers,
            body: &call.body,
        };
        Ok(self.clients.signed_request_with(request).await?)
    }

    async fn caller_identity(&self) -> Result<CallerIdentity, ProvisionError<Transport::Error>> {
        let call = query::caller_identity(&self.clients.settings.aws_region_name);
        let response = self.send(&call).await?.require_success(call.operation)?;
        Ok(query::caller_identity_from(&response.body)?)
    }
}

// a file is stored only when both its bytes and its name are present and non-empty
fn stored_file<'request>(
    bytes: Option<&'request [u8]>,
    name: Option<&'request str>,
) -> Option<(&'request [u8], &'request str)> {
    let content = bytes.filter(|value| !value.is_empty())?;
    let filename = name.filter(|value| !value.is_empty())?;
    Some((content, filename))
}
