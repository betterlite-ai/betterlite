use std::error::Error as StdError;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::aws_partition::get_aws_partition;
use serde::Serialize;
use tokio::time::sleep;

use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_ingestion::error::BedrockIngestionError;
use crate::bedrock_provisioning::calls::aoss;
use crate::bedrock_provisioning::calls::aoss::CollectionSelection;
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::plan;
use crate::bedrock_provisioning::plan::RoleResources;
use crate::bedrock_provisioning::provision::BedrockProvisioner;
use crate::bedrock_provisioning::provision::error::ProvisionError;

mod iam;
mod index;

const COLLECTION_POLLS: usize = 60;
const COLLECTION_POLL_INTERVAL: Duration = Duration::from_secs(5);
const COLLECTION_PROPAGATION: Duration = Duration::from_secs(60);
const ROLE_PROPAGATION: Duration = Duration::from_secs(10);
const ACTIVE: &str = "ACTIVE";

#[async_trait]
pub(crate) trait ProvisioningSteps {
    type Failure: StdError + Send + Sync + 'static;

    async fn create_opensearch_collection(
        &mut self,
        unique_id: &str,
        account_id: &str,
        caller_arn: &str,
    ) -> Result<(String, String), ProvisionError<Self::Failure>>;

    async fn create_opensearch_index(
        &self,
        collection_name: &str,
    ) -> Result<(), ProvisionError<Self::Failure>>;

    async fn create_bedrock_role(
        &mut self,
        unique_id: &str,
        account_id: &str,
        collection_arn: &str,
    ) -> Result<String, ProvisionError<Self::Failure>>;
}

#[async_trait]
impl<Transport> ProvisioningSteps for BedrockProvisioner<'_, Transport>
where
    Transport: AwsRagTransport,
{
    type Failure = Transport::Error;

    async fn create_opensearch_collection(
        &mut self,
        unique_id: &str,
        account_id: &str,
        caller_arn: &str,
    ) -> Result<(String, String), ProvisionError<Transport::Error>> {
        let name = format!("litellm-kb-{unique_id}");
        create_policies(self, &name, account_id, caller_arn).await?;
        let collection_id = create_collection(self, &name).await?;
        self.state.created_resources.opensearch_collection = Some(name.clone());
        let arn = wait_for_collection(self, &collection_id).await?;
        sleep(COLLECTION_PROPAGATION).await;
        Ok((name, arn))
    }

    async fn create_opensearch_index(
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

    async fn create_bedrock_role(
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
        let resources = RoleResources {
            arn_prefix: get_aws_partition(Some(region.as_str())).arn_prefix,
            region: &region,
            account_id,
            embedding_model: &embedding_model,
            collection_arn,
            bucket: &bucket,
        };
        let role_arn = iam::create_role(self, &resources, &role_name).await?;
        self.state.created_resources.iam_role = Some(role_name.clone());
        iam::attach_role_policy(self, &resources, &role_name).await?;
        sleep(ROLE_PROPAGATION).await;
        Ok(role_arn)
    }
}

async fn create_policies<Transport>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    name: &str,
    account_id: &str,
    caller_arn: &str,
) -> Result<(), ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let region = provisioner.clients.settings.aws_region_name.clone();
    let arn_prefix = get_aws_partition(Some(region.as_str())).arn_prefix;
    let encryption = plan::encryption_policy(name);
    create_policy(
        provisioner,
        aoss::CREATE_SECURITY_POLICY,
        &format!("{name}-enc"),
        aoss::ENCRYPTION,
        &encryption,
    )
    .await?;
    let network = plan::network_policy(name);
    create_policy(
        provisioner,
        aoss::CREATE_SECURITY_POLICY,
        &format!("{name}-net"),
        aoss::NETWORK,
        &network,
    )
    .await?;
    let access = plan::data_access_policy(name, arn_prefix, account_id, caller_arn);
    create_policy(
        provisioner,
        aoss::CREATE_ACCESS_POLICY,
        &format!("{name}-access"),
        aoss::DATA,
        &access,
    )
    .await
}

async fn create_policy<Transport, Policy>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    operation: &'static str,
    name: &str,
    kind: &str,
    policy: &Policy,
) -> Result<(), ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
    Policy: Serialize + Sync,
{
    let region = provisioner.clients.settings.aws_region_name.clone();
    let call = aoss::create_policy(&region, operation, name, kind, policy)?;
    provisioner
        .send(&call)
        .await?
        .require_success(call.operation)?;
    Ok(())
}

async fn create_collection<Transport>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    name: &str,
) -> Result<String, ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let region = provisioner.clients.settings.aws_region_name.clone();
    let call = aoss::create_collection(&region, name)?;
    let response = provisioner
        .send(&call)
        .await?
        .require_success(call.operation)?;
    let created: aoss::CreateCollectionResponse =
        serde_json::from_slice(&response.body).map_err(|source| CallError::Json {
            operation: aoss::CREATE_COLLECTION,
            source,
        })?;
    let id = created
        .create_collection_detail
        .and_then(|detail| detail.id)
        .ok_or(CallError::MissingEntry {
            operation: aoss::CREATE_COLLECTION,
            field: "createCollectionDetail.id",
        })?;
    Ok(id)
}

async fn wait_for_collection<Transport>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    collection_id: &str,
) -> Result<String, ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let region = provisioner.clients.settings.aws_region_name.clone();
    for _ in 0..COLLECTION_POLLS {
        let call = aoss::batch_get_collection(&region, CollectionSelection::Id(collection_id))?;
        let response = provisioner
            .send(&call)
            .await?
            .require_success(call.operation)?;
        let detail = aoss::first_collection(&response.body, aoss::BATCH_GET_COLLECTION)?;
        if detail.status.as_deref() == Some(ACTIVE) {
            let arn = detail.arn.ok_or(CallError::MissingEntry {
                operation: aoss::BATCH_GET_COLLECTION,
                field: "arn",
            })?;
            return Ok(arn);
        }
        sleep(COLLECTION_POLL_INTERVAL).await;
    }
    Err(ProvisionError::CollectionTimeout)
}
