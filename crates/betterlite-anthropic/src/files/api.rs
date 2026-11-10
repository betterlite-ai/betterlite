use std::str;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::files::records::{BinaryContent, FileDeleted, FileObject};
use betterlite_openai::files::stream::flatten_headers;
use betterlite_openai::files::transport::FileTransport;
use http::Method;

use super::error::FilesError;
use super::request::{FileCall, FilesClient, ListCall, Upload};
use super::wire::{decode_anthropic_delete, decode_anthropic_file, decode_anthropic_file_list};

type Reply<R> = Result<R, FilesError>;
type FileList = Vec<FileObject>;
type Client = FilesClient;

// A non-2xx reply is the SDK status error: the same status, message and headers reach B567's record
#[must_use]
pub fn response_error(
    status: u16,
    message: String,
    headers: Vec<(String, String)>,
) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code: status,
        message,
        headers: Some(headers),
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}

pub struct FilesApi<T> {
    pub transport: T,
    pub clock: fn() -> i64,
}

impl<T: FileTransport> FilesApi<T> {
    pub async fn create_file(&self, client: &Client, call: &Upload<'_>) -> Reply<FileObject> {
        let reply = self
            .reply(client.create_request(call, (self.clock)())?)
            .await?;
        decode_anthropic_file(&reply.body, (self.clock)())
    }

    pub async fn retrieve_file(&self, client: &Client, call: &FileCall<'_>) -> Reply<FileObject> {
        let reply = self.reply(client.file_request(Method::GET, call)?).await?;
        let file = decode_anthropic_file(&reply.body, (self.clock)())?;
        Ok(file)
    }

    pub async fn delete_file(&self, client: &Client, call: &FileCall<'_>) -> Reply<FileDeleted> {
        let reply = self
            .reply(client.file_request(Method::DELETE, call)?)
            .await?;
        decode_anthropic_delete(&reply.body)
    }

    pub async fn list_files(&self, client: &Client, call: &ListCall<'_>) -> Reply<FileList> {
        let reply = self.reply(client.list_request(call)?).await?;
        let files = decode_anthropic_file_list(&reply.body, (self.clock)())?;
        Ok(files)
    }

    pub async fn content(&self, client: &Client, call: &FileCall<'_>) -> Reply<BinaryContent> {
        let reply = self.reply(client.content_request(call)?).await?;
        let content = content_reply(reply)?;
        Ok(content)
    }

    async fn reply(&self, request: OutboundRequest) -> Result<UpstreamResponse, FilesError> {
        self.transport
            .send_buffered(request)
            .await
            .map_err(|source| FilesError::Transport { source })
    }
}

fn content_reply(reply: UpstreamResponse) -> Reply<BinaryContent> {
    if reply.status >= 400 {
        let headers: Vec<(String, String)> = flatten_headers(&reply.headers).into_iter().collect();
        let message = error_message(&reply.body);
        let error = response_error(reply.status, message, headers);
        return Err(FilesError::Provider(Box::new(error)));
    }
    Ok(BinaryContent::from_response(reply))
}

fn error_message(body: &[u8]) -> String {
    str::from_utf8(body).map_or_else(
        |_invalid| format!("the error body is {} bytes that are not UTF-8", body.len()),
        str::to_owned,
    )
}
