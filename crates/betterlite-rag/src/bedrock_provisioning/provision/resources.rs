use std::time::Duration;

use betterlite_context::aws_partition::get_aws_partition;
use serde::Serialize;
use tokio::time::sleep;
use url::Url;

use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_ingestion::error::{BedrockIngestionError, aws_error_body};
use crate::bedrock_provisioning::calls::CallRequest;
use crate::bedrock_provisioning::calls::aoss::{self, CollectionSelection};
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::calls::query;
use crate::bedrock_provisioning::plan::{self, RoleResources};
use crate::bedrock_provisioning::provision::BedrockProvisioner;
use crate::bedrock_provisioning::provision::error::ProvisionError;

const COLLECTION_POLLS: usize = 60;
const COLLECTION_POLL_INTERVAL: Duration = Duration::from_secs(5);
const COLLECTION_PROPAGATION: Duration = Duration::from_secs(60);
const INDEX_ATTEMPTS: usize = 8;
const INDEX_RETRY_DELAY: Duration = Duration::from_secs(20);
const ROLE_PROPAGATION: Duration = Duration::from_secs(10);
const ACTIVE: &str = "ACTIVE";
const INDEX_NAME: &str = "bedrock-kb-index";
const INDEX_OPERATION: &str = "CreateIndex";
const AUTHORIZATION_MARKERS: [&str; 2] = ["authorization_exception", "security_exception"];

impl<Transport> BedrockProvisioner<'_, Transport>
where
    Transport: AwsRagTransport,
{
    pub async fn create_opensearch_collection(
        &mut self,
        unique_id: &str,
        account_id: &str,
        caller_arn: &str,
    ) -> Result<(String, String), ProvisionError<Transport::Error>> {
        let region = self.clients.settings.aws_region_name.clone();
        let arn_prefix = get_aws_partition(Some(region.as_str())).arn_prefix;
        let name = format!("litellm-kb-{unique_id}");
        let encryption = plan::encryption_policy(&name);
        self.create_policy(
            &region,
            aoss::CREATE_SECURITY_POLICY,
            &format!("{name}-enc"),
            aoss::ENCRYPTION,
            &encryption,
        )
        .await?;
        let network = plan::network_policy(&name);
        self.create_policy(
            &region,
            aoss::CREATE_SECURITY_POLICY,
            &format!("{name}-net"),
            aoss::NETWORK,
            &network,
        )
        .await?;
        let access = plan::data_access_policy(&name, arn_prefix, account_id, caller_arn);
        self.create_policy(
            &region,
            aoss::CREATE_ACCESS_POLICY,
            &format!("{name}-access"),
            aoss::DATA,
            &access,
        )
        .await?;
        let call = aoss::create_collection(&region, &name)?;
        let response = self.send(&call).await?.require_success(call.operation)?;
        let created: aoss::CreateCollectionResponse =
            aoss::parse_json(&response.body, aoss::CREATE_COLLECTION)?;
        let collection_id = created
            .create_collection_detail
            .and_then(|detail| detail.id)
            .ok_or(CallError::MissingEntry {
                operation: aoss::CREATE_COLLECTION,
                field: "createCollectionDetail.id",
            })?;
        self.state.created_resources.opensearch_collection = Some(name.clone());
        let arn = self.wait_for_collection(&collection_id).await?;
        sleep(COLLECTION_PROPAGATION).await;
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
        let url = index_url(&endpoint, &region)?;
        let body = serde_json::to_vec(&plan::index::index_body()).map_err(|source| {
            CallError::Serialize {
                operation: INDEX_OPERATION,
                source,
            }
        })?;
        let mut last_status = 0_u16;
        let mut last_body = Vec::new();
        for _ in 0..INDEX_ATTEMPTS {
            let request = index_request(&region, &url, body.clone());
            let response = self.send(&request).await?;
            if (200..300).contains(&response.status) {
                return Ok(());
            }
            if !is_authorization_failure(&response.body) {
                let failure = BedrockIngestionError::Aws {
                    operation: INDEX_OPERATION,
                    status: response.status,
                    body: aws_error_body(&response.body),
                };
                return Err(ProvisionError::from(failure));
            }
            last_status = response.status;
            last_body = response.body;
            sleep(INDEX_RETRY_DELAY).await;
        }
        Err(ProvisionError::IndexRetriesExhausted {
            attempts: INDEX_ATTEMPTS,
            status: last_status,
            body: aws_error_body(&last_body),
        })
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
        let resources = RoleResources {
            arn_prefix: get_aws_partition(Some(region.as_str())).arn_prefix,
            region: &region,
            account_id,
            embedding_model: &embedding_model,
            collection_arn,
            bucket: &bucket,
        };
        let role_arn = self.create_role(&resources, &role_name).await?;
        self.state.created_resources.iam_role = Some(role_name.clone());
        self.attach_role_policy(&resources, &role_name).await?;
        sleep(ROLE_PROPAGATION).await;
        Ok(role_arn)
    }

    async fn create_role(
        &self,
        resources: &RoleResources<'_>,
        role_name: &str,
    ) -> Result<String, ProvisionError<Transport::Error>> {
        let document = plan_document(&plan::trust_policy(resources), query::CREATE_ROLE)?;
        let body = self
            .iam_call(
                query::CREATE_ROLE,
                resources.region,
                &[
                    ("RoleName", role_name),
                    ("AssumeRolePolicyDocument", document.as_str()),
                ],
            )
            .await?;
        query::child_text(&body, "Role", "Arn", query::CREATE_ROLE)
    }

    async fn attach_role_policy(
        &self,
        resources: &RoleResources<'_>,
        role_name: &str,
    ) -> Result<(), ProvisionError<Transport::Error>> {
        let document = plan_document(&plan::permissions_policy(resources), query::PUT_ROLE_POLICY)?;
        let policy_name = format!("{role_name}-policy");
        let mut pairs: Vec<(&str, &str)> = vec![("RoleName", role_name)];
        pairs.push(("PolicyName", policy_name.as_str()));
        pairs.push(("PolicyDocument", document.as_str()));
        self.iam_call(query::PUT_ROLE_POLICY, resources.region, &pairs)
            .await?;
        Ok(())
    }

    async fn iam_call(
        &self,
        operation: &'static str,
        region: &str,
        pairs: &[(&str, &str)],
    ) -> Result<Vec<u8>, ProvisionError<Transport::Error>> {
        let call = query::iam_request(region, operation, pairs);
        let response = self.send(&call).await?.require_success(call.operation)?;
        Ok(response.body)
    }

    async fn create_policy<Policy: Serialize>(
        &self,
        region: &str,
        operation: &'static str,
        name: &str,
        kind: &str,
        policy: &Policy,
    ) -> Result<(), ProvisionError<Transport::Error>> {
        let call = aoss::create_policy(region, operation, name, kind, policy)?;
        self.send(&call).await?.require_success(call.operation)?;
        Ok(())
    }

    async fn wait_for_collection(
        &self,
        collection_id: &str,
    ) -> Result<String, ProvisionError<Transport::Error>> {
        let region = self.clients.settings.aws_region_name.clone();
        for _ in 0..COLLECTION_POLLS {
            let call = aoss::batch_get_collection(&region, CollectionSelection::Id(collection_id))?;
            let response = self.send(&call).await?.require_success(call.operation)?;
            let detail = aoss::first_collection(&response.body, aoss::BATCH_GET_COLLECTION)?;
            if detail.status.as_deref() == Some(ACTIVE) {
                return detail.arn.ok_or(CallError::MissingEntry {
                    operation: aoss::BATCH_GET_COLLECTION,
                    field: "arn",
                });
            }
            sleep(COLLECTION_POLL_INTERVAL).await;
        }
        Err(ProvisionError::CollectionTimeout)
    }
}

fn plan_document<Plan: Serialize>(
    plan: &Plan,
    operation: &'static str,
) -> Result<String, CallError> {
    serde_json::to_string(plan).map_err(|source| CallError::Serialize { operation, source })
}

fn index_url(endpoint: &str, region: &str) -> Result<String, CallError> {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    let parsed = Url::parse(endpoint).map_err(|_source| CallError::CollectionEndpoint {
        endpoint: endpoint.to_owned(),
    })?;
    let on_aoss = parsed
        .host_str()
        .is_some_and(|host| host.ends_with(&format!(".aoss.{suffix}")));
    if parsed.scheme() != "https" || !on_aoss {
        return Err(CallError::CollectionEndpoint {
            endpoint: endpoint.to_owned(),
        });
    }
    Ok(format!("{}/{}", endpoint.trim_end_matches('/'), INDEX_NAME))
}

fn index_request(region: &str, url: &str, body: Vec<u8>) -> CallRequest {
    CallRequest {
        operation: INDEX_OPERATION,
        service: aoss::AOSS_SERVICE,
        signing_region: region.to_owned(),
        method: "PUT",
        url: url.to_owned(),
        headers: vec![(
            "Content-Type".to_owned(),
            aoss::JSON_CONTENT_TYPE.to_owned(),
        )],
        body,
    }
}

fn is_authorization_failure(body: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    let lowered = text.to_lowercase();
    AUTHORIZATION_MARKERS
        .iter()
        .any(|marker| lowered.contains(*marker))
}
