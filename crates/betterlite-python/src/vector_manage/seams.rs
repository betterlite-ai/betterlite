use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_core::api::vector_store_http_create_read::config::{
    CreateReadConfig, OpenAiCreateReadConfig,
};
use betterlite_core::api::vector_store_read::service::{ReadConfig, VectorStoreReadConfigs};
use betterlite_core::api::vector_store_search::provider::ResolvedProvider;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::SecretString;

mod tests;

// B010's read config for the tags this build serves
pub(crate) struct Configs<'core> {
    pub(crate) credentials: &'core OpenAiCredentialsSettings,
    pub(crate) globals: &'core LiteLlmGlobals,
    pub(crate) env_api_key: &'core Option<SecretString>,
}

impl VectorStoreReadConfigs for Configs<'_> {
    fn read_config<'source>(
        &'source self,
        resolved: &ResolvedProvider,
    ) -> Option<ReadConfig<'source>> {
        if !served(resolved.tag) {
            return None;
        }
        let config: Box<dyn CreateReadConfig + 'source> = Box::new(OpenAiCreateReadConfig::new(
            self.credentials,
            globals(self.globals),
            self.env_api_key.clone(),
            self.globals.api_base.clone(),
        ));
        Some(ReadConfig(config))
    }
}

// B010 serves OpenAI's vector stores only; the other tags are named so a new provider fails to compile here
const fn served(tag: VectorStoresConfigTag) -> bool {
    match tag {
        VectorStoresConfigTag::OpenAI => true,
        VectorStoresConfigTag::AzureOpenAI
        | VectorStoresConfigTag::Vertex
        | VectorStoresConfigTag::VertexSearchAPI
        | VectorStoresConfigTag::Bedrock
        | VectorStoresConfigTag::PGVector
        | VectorStoresConfigTag::AzureAI
        | VectorStoresConfigTag::Milvus
        | VectorStoresConfigTag::Gemini
        | VectorStoresConfigTag::RAGFlow
        | VectorStoresConfigTag::S3Vectors
        | VectorStoresConfigTag::Valkey
        | VectorStoresConfigTag::MongoDB => false,
    }
}

// B527 owns the credential source rules, so the globals are copied into the shape its resolver reads
pub(crate) fn globals(source: &LiteLlmGlobals) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: source.api_base.clone(),
        api_key: source.api_key.clone(),
        openai_key: source.openai_key.clone(),
        organization: source.organization.clone(),
    }
}
