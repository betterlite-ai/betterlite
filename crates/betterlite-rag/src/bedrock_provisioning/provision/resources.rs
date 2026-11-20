use std::time::Duration;

use betterlite_context::aws_partition::get_aws_partition;
use serde::Serialize;
use tokio::time::sleep;

use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_provisioning::calls::aoss;
use crate::bedrock_provisioning::calls::aoss::CollectionSelection;
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::plan;
use crate::bedrock_provisioning::provision::BedrockProvisioner;
use crate::bedrock_provisioning::provision::error::ProvisionError;

const COLLECTION_POLLS: usize = 60;
const COLLECTION_POLL_INTERVAL: Duration = Duration::from_secs(5);
const COLLECTION_PROPAGATION: Duration = Duration::from_secs(60);
const ACTIVE: &str = "ACTIVE";

pub(super) async fn create_policies<Transport>(
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

// record the collection before the wait, so cleanup finds it even when the wait times out
pub(super) async fn create_collection_and_wait<Transport>(
    provisioner: &mut BedrockProvisioner<'_, Transport>,
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
    let collection_id = created
        .create_collection_detail
        .and_then(|detail| detail.id)
        .ok_or(CallError::MissingEntry {
            operation: aoss::CREATE_COLLECTION,
            field: "createCollectionDetail.id",
        })?;
    provisioner.state.created_resources.opensearch_collection = Some(name.to_owned());
    for _ in 0..COLLECTION_POLLS {
        let poll_call =
            aoss::batch_get_collection(&region, CollectionSelection::Id(&collection_id))?;
        let poll_response = provisioner
            .send(&poll_call)
            .await?
            .require_success(poll_call.operation)?;
        let detail = aoss::first_collection(&poll_response.body, aoss::BATCH_GET_COLLECTION)?;
        if detail.status.as_deref() == Some(ACTIVE) {
            let arn = detail.arn.ok_or(CallError::MissingEntry {
                operation: aoss::BATCH_GET_COLLECTION,
                field: "arn",
            })?;
            sleep(COLLECTION_PROPAGATION).await;
            return Ok(arn);
        }
        sleep(COLLECTION_POLL_INTERVAL).await;
    }
    Err(ProvisionError::CollectionTimeout)
}
