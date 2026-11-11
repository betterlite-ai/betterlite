use std::time::Duration;

use betterlite_context::aws_partition::get_aws_partition;
use sha2::{Digest as _, Sha256};
use tokio::time::sleep;
use uuid::Uuid;

use crate::bedrock_ingestion::client::endpoints::{create_bucket_body, s3_bucket_host};
use crate::bedrock_ingestion::client::{
    AwsRagTransport, BedrockRagClients, S3_SERVICE, SignedRequest,
};
use crate::bedrock_ingestion::error::BedrockIngestionError;
use crate::bedrock_ingestion::settings::{BedrockRagSettings, DEFAULT_S3_PREFIX};
use crate::bedrock_ingestion::state::{BedrockRagState, CreatedResources};
use crate::bedrock_ingestion::wire::{
    CreateDataSourceRequest, DataSourceType, GetDataSourceResponse, NewDataSourceConfiguration,
    NewS3DataSourceConfiguration,
};

mod knowledge_base;
pub mod principal;
mod tests;

const KNOWLEDGE_BASE_POLLS: usize = 30;
const KNOWLEDGE_BASE_POLL_INTERVAL: Duration = Duration::from_secs(2);
const ACTIVE: &str = "ACTIVE";

impl BedrockRagState {
    #[must_use]
    pub fn new(settings: &BedrockRagSettings) -> Self {
        let s3_prefix = settings
            .s3_prefix
            .clone()
            .unwrap_or_else(|| DEFAULT_S3_PREFIX.to_owned());
        Self {
            data_source_id: None,
            s3_bucket: None,
            s3_prefix,
            created_resources: CreatedResources::default(),
        }
    }

    pub async fn auto_detect_config<Transport>(
        &mut self,
        clients: &BedrockRagClients<'_, Transport>,
    ) -> Result<(), BedrockIngestionError<Transport::Error>>
    where
        Transport: AwsRagTransport,
    {
        let knowledge_base_id = clients
            .settings
            .knowledge_base_id
            .clone()
            .ok_or(BedrockIngestionError::MissingKnowledgeBaseId)?;
        let data_source_id = detect_data_source(self, clients, &knowledge_base_id).await?;
        let details = clients
            .get_data_source(&knowledge_base_id, &data_source_id)
            .await?;
        apply_s3_configuration(self, clients, &data_source_id, &details)
    }

    pub async fn create_s3_bucket<Transport>(
        &mut self,
        clients: &BedrockRagClients<'_, Transport>,
        unique_id: &str,
    ) -> Result<String, BedrockIngestionError<Transport::Error>>
    where
        Transport: AwsRagTransport,
    {
        let bucket_name = format!("litellm-kb-{unique_id}");
        create_bucket(clients, &bucket_name).await?;
        self.created_resources.s3_bucket = Some(bucket_name.clone());
        Ok(bucket_name)
    }

    pub async fn create_knowledge_base<Transport>(
        &mut self,
        clients: &BedrockRagClients<'_, Transport>,
        kb_name: &str,
        role_arn: &str,
        collection_arn: &str,
    ) -> Result<String, BedrockIngestionError<Transport::Error>>
    where
        Transport: AwsRagTransport,
    {
        let request = knowledge_base::create_request(
            &clients.settings.aws_region_name,
            &clients.settings.embedding_model,
            kb_name,
            role_arn,
            collection_arn,
        );
        let created = clients.create_knowledge_base(&request).await?;
        let knowledge_base_id = created.knowledge_base.knowledge_base_id;
        self.created_resources.knowledge_base = Some(knowledge_base_id.clone());
        for _ in 0..KNOWLEDGE_BASE_POLLS {
            let state = clients.get_knowledge_base(&knowledge_base_id).await?;
            if state.knowledge_base.status == ACTIVE {
                return Ok(knowledge_base_id);
            }
            sleep(KNOWLEDGE_BASE_POLL_INTERVAL).await;
        }
        Err(BedrockIngestionError::KnowledgeBaseTimeout)
    }

    pub async fn create_data_source<Transport>(
        &mut self,
        clients: &BedrockRagClients<'_, Transport>,
        kb_name: &str,
    ) -> Result<String, BedrockIngestionError<Transport::Error>>
    where
        Transport: AwsRagTransport,
    {
        let knowledge_base_id = self
            .created_resources
            .knowledge_base
            .clone()
            .or_else(|| clients.settings.knowledge_base_id.clone())
            .ok_or(BedrockIngestionError::MissingKnowledgeBaseId)?;
        let bucket = self
            .s3_bucket
            .clone()
            .ok_or(BedrockIngestionError::MissingS3Bucket)?;
        let request = data_source_request(
            &clients.settings.aws_region_name,
            kb_name,
            &bucket,
            &self.s3_prefix,
        );
        let created = clients
            .create_data_source(&knowledge_base_id, &request)
            .await?;
        let data_source_id = created.data_source.data_source_id;
        self.created_resources.data_source = Some(data_source_id.clone());
        Ok(data_source_id)
    }
}

async fn detect_data_source<Transport>(
    state: &mut BedrockRagState,
    clients: &BedrockRagClients<'_, Transport>,
    knowledge_base_id: &str,
) -> Result<String, BedrockIngestionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let listed = clients.list_data_sources(knowledge_base_id).await?;
    let first = listed.data_source_summaries.first().ok_or_else(|| {
        BedrockIngestionError::NoDataSources {
            knowledge_base_id: knowledge_base_id.to_owned(),
        }
    })?;
    let data_source_id = clients
        .settings
        .data_source_id
        .clone()
        .unwrap_or_else(|| first.data_source_id.clone());
    state.data_source_id = Some(data_source_id.clone());
    Ok(data_source_id)
}

fn apply_s3_configuration<Transport>(
    state: &mut BedrockRagState,
    clients: &BedrockRagClients<'_, Transport>,
    data_source_id: &str,
    details: &GetDataSourceResponse,
) -> Result<(), BedrockIngestionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let s3 = details
        .data_source
        .data_source_configuration
        .as_ref()
        .and_then(|configuration| configuration.s3_configuration.as_ref());
    let bucket_arn = s3
        .and_then(|configuration| configuration.bucket_arn.clone())
        .unwrap_or_default();
    if bucket_arn.is_empty() {
        let configured = clients.settings.s3_bucket.clone().ok_or_else(|| {
            BedrockIngestionError::NoS3Bucket {
                data_source_id: data_source_id.to_owned(),
            }
        })?;
        state.s3_bucket = Some(configured);
        return Ok(());
    }
    let arn_bucket = bucket_arn.rsplit(':').next().unwrap_or_default();
    state.s3_bucket = Some(
        clients
            .settings
            .s3_bucket
            .clone()
            .unwrap_or_else(|| arn_bucket.to_owned()),
    );
    let prefixes = s3.and_then(|configuration| configuration.inclusion_prefixes.as_ref());
    if clients.settings.s3_prefix.is_none()
        && let Some(prefix) = prefixes.and_then(|values| values.first())
    {
        state.s3_prefix.clone_from(prefix);
    }
    Ok(())
}

async fn create_bucket<Transport>(
    clients: &BedrockRagClients<'_, Transport>,
    bucket_name: &str,
) -> Result<(), BedrockIngestionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let region = clients.settings.aws_region_name.clone();
    let body = create_bucket_body(&region);
    let bytes = body.as_deref().unwrap_or_default().as_bytes();
    let headers = vec![(
        "x-amz-content-sha256".to_owned(),
        hex::encode(Sha256::digest(bytes)),
    )];
    let url = format!("{}/", s3_bucket_host(bucket_name, &region));
    let request = SignedRequest {
        service: S3_SERVICE,
        signing_region: region.as_str(),
        method: "PUT",
        url: &url,
        headers: &headers,
        body: bytes,
    };
    clients
        .signed_request_with(request)
        .await?
        .require_success("CreateBucket")?;
    Ok(())
}

fn data_source_request(
    region: &str,
    kb_name: &str,
    bucket: &str,
    prefix: &str,
) -> CreateDataSourceRequest {
    let arn_prefix = get_aws_partition(Some(region)).arn_prefix;
    CreateDataSourceRequest {
        client_token: Uuid::new_v4().to_string(),
        name: format!("{kb_name}-s3-source"),
        data_source_configuration: NewDataSourceConfiguration {
            data_source_type: DataSourceType::S3,
            s3_configuration: NewS3DataSourceConfiguration {
                bucket_arn: format!("{arn_prefix}s3:::{bucket}"),
                inclusion_prefixes: vec![prefix.to_owned()],
            },
        },
    }
}
