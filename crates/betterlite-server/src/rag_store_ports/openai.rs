use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_core::api::vector_file_http_create_list::config::{
    FilesConfig, FilesParams, OpenAiFilesConfig,
};
use betterlite_core::api::vector_store_http_create_read::config::{
    CreateReadConfig, OpenAiCreateReadConfig,
};
use betterlite_core::api::vector_store_http_write::config::{WriteConfig, WriteParams};
use betterlite_core::api::vector_store_http_write::error::header_map;
use betterlite_core::rag::openai_ingestion::{
    AttachFile, CreateStore, PortError, VectorFilePort, VectorStorePort,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::files::create_request as file_create;
use betterlite_openai::vector_stores::store::create_request as store_create;

use self::wire::{
    OpenAiPortError, file_params, openai_headers, port_error, post, reply_text, status_error,
    store_id, store_params,
};

mod tests;
mod upload;
mod wire;

// the store create over B1064's native request and reply transforms
pub struct OpenAiStorePort {
    pub client: Arc<ProxiedClient>,
}

// the vector file create over B1072's native request and reply transforms
pub struct OpenAiFilePort {
    pub client: Arc<ProxiedClient>,
}

// the file create over B1028's native FilesApi, with the SDK's own client checks
pub struct OpenAiUploadPort {
    pub client: Arc<ProxiedClient>,
}

impl Debug for OpenAiStorePort {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("OpenAiStorePort").finish_non_exhaustive()
    }
}

impl Debug for OpenAiFilePort {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("OpenAiFilePort").finish_non_exhaustive()
    }
}

impl Debug for OpenAiUploadPort {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("OpenAiUploadPort").finish_non_exhaustive()
    }
}

#[async_trait]
impl VectorStorePort for OpenAiStorePort {
    async fn create_store(&self, request: CreateStore<'_>) -> Result<Option<String>, PortError> {
        let settings = OpenAiCredentialsSettings::default();
        let provider =
            OpenAiCreateReadConfig::new(&settings, LiteLlmGlobals::default(), None, None);
        let config = provider.provider();
        let params = WriteParams {
            api_base: request.api_base.map(str::to_owned),
            api_key: request.api_key.cloned(),
            ssl_verify: None,
        };
        let url = config.complete_url(&params).map_err(port_error)?;
        let headers = openai_headers(config.validate_environment(BTreeMap::new(), &params))
            .map_err(port_error)?;
        let body = store_create(&store_params(&request), &url).body;
        let payload = serde_json::to_vec(&body)
            .map_err(|source| port_error(OpenAiPortError::Encode { source }))?;
        let reply = post(&self.client, &url, headers, RequestBody::Json(payload)).await?;
        if !(200..300).contains(&reply.status) {
            return Err(status_error(config, &reply));
        }
        store_id(config, &reply)
    }
}

#[async_trait]
impl VectorFilePort for OpenAiFilePort {
    async fn attach(&self, request: AttachFile<'_>) -> Result<(), PortError> {
        let settings = OpenAiCredentialsSettings::default();
        let provider = OpenAiFilesConfig::new(&settings, LiteLlmGlobals::default(), None, None);
        let params = FilesParams {
            api_base: request.api_base.map(str::to_owned),
            api_key: request.api_key.cloned(),
            ssl_verify: None,
        };
        let url = provider
            .complete_url(&params, request.vector_store_id)
            .map_err(port_error)?;
        let headers = openai_headers(provider.validate_environment(BTreeMap::new(), &params))
            .map_err(port_error)?;
        let body = file_create(&file_params(&request).map_err(port_error)?, &url).body;
        let payload = serde_json::to_vec(&body)
            .map_err(|source| port_error(OpenAiPortError::Encode { source }))?;
        let reply = post(&self.client, &url, headers, RequestBody::Json(payload)).await?;
        if reply.status >= 400 {
            let message = reply_text(&reply.body);
            return Err(port_error(provider.error_class(
                &message,
                reply.status,
                header_map(&reply.headers),
            )));
        }
        let _decoded = provider.transform_response(&reply).map_err(port_error)?;
        Ok(())
    }
}
