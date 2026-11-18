pub mod error;
pub mod execute;
mod hooks;
pub mod plan;
pub mod sign;

mod tests;

use std::collections::BTreeMap;
use std::future::Future;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::decode::LegacyEmbeddingResponse;
use betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::credentials::ManagedRegionWorker;
use crate::bedrock_embedding_io::response::EmbeddingReply;
use crate::bedrock_embedding_io::transport::{ClientFactory, SuppliedClient, Timeout};
use crate::credentials::error::CredentialError;
use crate::signing::executor::AwsSigningExecutor;
use error::EmbeddingCallError;

pub type ParamMap = BTreeMap<String, Box<RawValue>>;

pub struct EmbeddingEnvironment<'text> {
    pub variables: &'text BTreeMap<String, String>,
    pub runtime_endpoint: Option<&'text str>,
    pub bearer_token: Option<&'text SecretString>,
}

pub struct EmbeddingCall<'text> {
    pub model: &'text str,
    pub input: &'text [String],
    pub optional_params: ParamMap,
    pub api_base: Option<&'text str>,
    pub extra_headers: Option<&'text [(String, String)]>,
    pub api_key: Option<&'text SecretString>,
    pub timeout: Option<Timeout>,
    pub aembedding: bool,
    pub drop_params: bool,
    pub environment: EmbeddingEnvironment<'text>,
    pub now: SystemTime,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub trait CredentialSource {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send;
}

pub struct Dependencies<'source, Source, Factory, Transport> {
    pub secrets: &'source SecretLookup,
    pub region_worker: &'source ManagedRegionWorker,
    pub source: &'source Source,
    pub factory: &'source Factory,
    pub transport: &'source Transport,
    pub encoding: &'source (dyn TextEncoding + Send + Sync),
    pub signing: &'source AwsSigningExecutor,
}

#[derive(Debug)]
pub enum EmbeddingCallResponse {
    Batch(EmbeddingReply),
    Legacy(LegacyEmbeddingResponse),
}

pub async fn embeddings<Source, Factory, Transport>(
    call: &mut EmbeddingCall<'_>,
    dependencies: Dependencies<'_, Source, Factory, Transport>,
    supplied: Option<SuppliedClient<'_, Factory::Client>>,
    log: &mut LogContext<'_, '_>,
) -> Result<EmbeddingCallResponse, EmbeddingCallError>
where
    Source: CredentialSource + Sync,
    Factory: ClientFactory + Sync,
    Factory::Client: LegacyEmbeddingTransport,
    Transport: LegacyEmbeddingTransport,
{
    let plan = plan::plan(
        call,
        dependencies.secrets,
        dependencies.region_worker,
        dependencies.source,
    )
    .await?;
    execute::execute(call, &plan, &dependencies, supplied, log).await
}
