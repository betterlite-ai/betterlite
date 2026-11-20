use async_trait::async_trait;

use super::S3VectorsApi;
use crate::bedrock_ingestion::client::clock::Clock;
use crate::bedrock_ingestion::client::credentials::RagCredentialResolver;
use crate::bedrock_ingestion::client::{AwsRagTransport, BedrockRagClients};
use crate::bedrock_ingestion::settings::{
    BedrockRagSettings, DEFAULT_EMBEDDING_MODEL, DEFAULT_INGESTION_TIMEOUT,
};
use crate::s3_vectors_ingestion::{
    S3VectorEntry, S3VectorsError, S3VectorsRequests, VectorIndexConfig,
};
use crate::s3_vectors_workflow::settings::S3VectorsSettings;

pub struct SignedS3Vectors<'deps, Transport> {
    clients: BedrockRagClients<'deps, Transport>,
    aws_region_name: String,
}

impl<'deps, Transport> SignedS3Vectors<'deps, Transport> {
    #[must_use]
    pub fn new(
        settings: &S3VectorsSettings,
        resolver: Box<dyn RagCredentialResolver + 'deps>,
        clock: Box<dyn Clock + 'deps>,
        transport: Transport,
    ) -> Self {
        Self {
            clients: BedrockRagClients {
                settings: bedrock_settings(settings),
                resolver,
                clock,
                transport,
            },
            aws_region_name: settings.aws_region_name.clone(),
        }
    }

    fn requests(&self) -> S3VectorsRequests<'_, 'deps, Transport> {
        S3VectorsRequests {
            clients: &self.clients,
            aws_region_name: self.aws_region_name.as_str(),
        }
    }
}

#[async_trait]
impl<Transport> S3VectorsApi for SignedS3Vectors<'_, Transport>
where
    Transport: AwsRagTransport,
{
    type Error = S3VectorsError<Transport::Error>;

    async fn ensure_bucket(&self, bucket_name: &str) -> Result<(), Self::Error> {
        self.requests()
            .ensure_vector_bucket_exists(bucket_name)
            .await
    }

    async fn ensure_index(&self, index: &VectorIndexConfig<'_>) -> Result<(), Self::Error> {
        self.requests().ensure_vector_index_exists(index).await
    }

    async fn put_vectors(
        &self,
        bucket_name: &str,
        index_name: &str,
        vectors: &[S3VectorEntry],
    ) -> Result<(), Self::Error> {
        self.requests()
            .put_vectors(bucket_name, index_name, vectors)
            .await
    }
}

fn bedrock_settings(settings: &S3VectorsSettings) -> BedrockRagSettings {
    BedrockRagSettings {
        knowledge_base_id: None,
        data_source_id: None,
        s3_bucket: None,
        s3_prefix: None,
        embedding_model: DEFAULT_EMBEDDING_MODEL.to_owned(),
        wait_for_ingestion: false,
        ingestion_timeout: DEFAULT_INGESTION_TIMEOUT,
        aws_region_name: settings.aws_region_name.clone(),
        credentials: settings.credentials.clone(),
    }
}
