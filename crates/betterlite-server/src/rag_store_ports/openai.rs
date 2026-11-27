use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt::{Debug, Formatter, Result as FmtResult};
use std::str::from_utf8;
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
    AttachFile, CreateStore, FileUploadPort, PortError, UploadFile, VectorFilePort, VectorStorePort,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreChunkingStrategy, VectorStoreExpirationPolicy,
};
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::files::records::{
    CreatePurpose, FileCreate, FileUpload as NativeFileUpload,
};
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use betterlite_openai::files::{ClientArgs, FilesApi, get_openai_client, select_client};
use betterlite_openai::vector_stores::files::{FileCreateParams, create_request as file_create};
use betterlite_openai::vector_stores::store::{CreateParams, create_request as store_create};
use http::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use http::{HeaderName, Method, StatusCode};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;
use uuid::Uuid;

use super::exchange;

mod tests;

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

#[async_trait]
impl FileUploadPort for OpenAiUploadPort {
    async fn upload(&self, request: UploadFile<'_>) -> Result<String, PortError> {
        let create = FileCreate {
            file: NativeFileUpload {
                filename: Some(request.filename.to_owned()),
                content_type: Some(request.content_type.to_owned()),
                data: request.content.to_vec(),
            },
            purpose: purpose(request.purpose)?,
            expires_after: None,
            extra_headers: IndexMap::new(),
            extra_body: IndexMap::new(),
            timeout: None,
        };
        let args = ClientArgs {
            api_key: request.api_key.cloned(),
            api_base: request.api_base.map(str::to_owned),
            timeout: None,
            max_retries: None,
            organization: None,
        };
        let client = select_client(
            get_openai_client(args, None, true).map_err(port_error)?,
            true,
        )
        .map_err(port_error)?;
        let api = FilesApi {
            transport: SharedFiles {
                client: &self.client,
            },
            boundaries: Uuid::new_v4,
        };
        let object = api
            .create_file(&client, &create)
            .await
            .map_err(port_error)?;
        Ok(object.id)
    }
}

// B1028's FilesApi takes a transport by value, so the shared RAG client comes in by reference
struct SharedFiles<'client> {
    client: &'client ProxiedClient,
}

impl FileTransport for SharedFiles<'_> {
    async fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        self.client.send_buffered(request).await
    }

    async fn send_stream(&self, request: OutboundRequest) -> Result<StreamReply, TransportError> {
        FileTransport::send_stream(self.client, request).await
    }
}

#[derive(Debug, thiserror::Error)]
enum OpenAiPortError {
    #[error("the api_base {url:?} is not a URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: http::header::InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    Header {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the request body could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the chunking_strategy is not a chunking strategy")]
    Chunking {
        #[source]
        source: serde_json::Error,
    },
    #[error("the file purpose {purpose:?} is not a purpose the file create accepts")]
    Purpose { purpose: String },
    #[error("the request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
}

fn port_error<Failure>(failure: Failure) -> PortError
where
    Failure: StdError + Send + Sync + 'static,
{
    PortError {
        message: failure.to_string(),
        source: Box::new(failure),
    }
}

async fn post(
    client: &ProxiedClient,
    url: &str,
    headers: HeaderMap,
    body: RequestBody,
) -> Result<UpstreamResponse, PortError> {
    let target = Url::parse(url).map_err(|source| {
        port_error(OpenAiPortError::Url {
            url: url.to_owned(),
            source,
        })
    })?;
    exchange(
        client,
        OutboundRequest {
            method: Method::POST,
            url: target,
            query: Vec::new(),
            headers,
            body,
            deadline: client.limits.read_timeout,
        },
    )
    .await
    .map_err(|source| port_error(OpenAiPortError::Transport { source }))
}

fn openai_headers(headers: BTreeMap<String, SecretString>) -> Result<HeaderMap, OpenAiPortError> {
    let mut mapped = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            OpenAiPortError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let mut header_value = HeaderValue::from_str(value.expose_secret())
            .map_err(|source| OpenAiPortError::Header { name, source })?;
        if header_name == AUTHORIZATION {
            header_value.set_sensitive(true);
        }
        mapped.insert(header_name, header_value);
    }
    Ok(mapped)
}

fn store_params(request: &CreateStore<'_>) -> CreateParams {
    CreateParams {
        name: Some(request.name.to_owned()),
        file_ids: None,
        expires_after: request
            .expires_after
            .map(|expires| VectorStoreExpirationPolicy {
                anchor: Some(expires.anchor.to_owned()),
                days: Some(expires.days),
                extra: BTreeMap::new(),
            }),
        chunking_strategy: None,
        metadata: None,
    }
}

fn file_params(request: &AttachFile<'_>) -> Result<FileCreateParams, OpenAiPortError> {
    let chunking_strategy = serde_json::from_str::<Option<VectorStoreChunkingStrategy>>(
        request.chunking_strategy.get(),
    )
    .map_err(|source| OpenAiPortError::Chunking { source })?;
    Ok(FileCreateParams {
        file_id: Some(request.file_id.to_owned()),
        attributes: None,
        chunking_strategy,
    })
}

fn purpose(purpose: &str) -> Result<CreatePurpose, PortError> {
    match purpose {
        "assistants" => Ok(CreatePurpose::Assistants),
        "batch" => Ok(CreatePurpose::Batch),
        "fine-tune" => Ok(CreatePurpose::FineTune),
        "messages" => Ok(CreatePurpose::Messages),
        other => Err(port_error(OpenAiPortError::Purpose {
            purpose: other.to_owned(),
        })),
    }
}

fn reply_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_error| format!("{} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}

fn status_error(provider: &dyn WriteConfig, reply: &UpstreamResponse) -> PortError {
    let message = reply_text(&reply.body);
    port_error(provider.error_class(&message, reply.status, header_map(&reply.headers)))
}

fn store_id(
    provider: &dyn WriteConfig,
    reply: &UpstreamResponse,
) -> Result<Option<String>, PortError> {
    let decoded = provider
        .transform_create_response(&http_response(reply))
        .map_err(port_error)?;
    let Json::Object(mut fields) = decoded else {
        return Ok(None);
    };
    match fields.shift_remove("id") {
        Some(Json::Text(id)) => Ok(Some(id)),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Ok(None)
        }
    }
}

fn http_response(reply: &UpstreamResponse) -> http::Response<Vec<u8>> {
    let mut response = http::Response::new(reply.body.clone());
    *response.status_mut() = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::OK);
    *response.headers_mut() = reply.headers.clone();
    response
}
