use async_trait::async_trait;
use betterlite_core::rag::openai_ingestion::{FileUploadPort, PortError, UploadFile};
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::files::records::{
    CreatePurpose, FileCreate, FileUpload as NativeFileUpload,
};
use betterlite_openai::files::transport::{FileTransport, StreamReply};
use betterlite_openai::files::{ClientArgs, FilesApi, get_openai_client, select_client};
use indexmap::IndexMap;
use uuid::Uuid;

use super::OpenAiUploadPort;
use super::wire::{OpenAiPortError, port_error};

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
