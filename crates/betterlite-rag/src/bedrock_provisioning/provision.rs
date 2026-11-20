use std::fmt::{Debug, Formatter, Result as FmtResult};

use crate::bedrock_ingestion::client::{
    AwsRagResponse, AwsRagTransport, BedrockRagClients, SignedRequest,
};
use crate::bedrock_ingestion::state::BedrockRagState;
use crate::bedrock_provisioning::calls::CallRequest;
use crate::bedrock_provisioning::calls::query::{self, CallerIdentity};
use crate::bedrock_provisioning::provision::error::ProvisionError;

pub mod error;
mod resources;
pub(crate) mod tests;

pub struct BedrockProvisioner<'deps, Transport> {
    pub clients: BedrockRagClients<'deps, Transport>,
    pub state: BedrockRagState,
    pub ingest_name: Option<String>,
    pub unique_id: String,
    pub initialized: bool,
}

impl<Transport> Debug for BedrockProvisioner<'_, Transport> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("BedrockProvisioner")
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

    pub(crate) async fn send_ok(
        &self,
        call: &CallRequest,
    ) -> Result<AwsRagResponse, ProvisionError<Transport::Error>> {
        let response = self.send(call).await?;
        Ok(response.require_success(call.operation)?)
    }

    async fn caller_identity(&self) -> Result<CallerIdentity, ProvisionError<Transport::Error>> {
        let call = query::caller_identity(&self.clients.settings.aws_region_name);
        let response = self.send_ok(&call).await?;
        Ok(query::caller_identity_from(&response.body)?)
    }
}
