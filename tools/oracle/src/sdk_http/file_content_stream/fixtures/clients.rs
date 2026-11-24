use std::collections::BTreeMap;

use betterlite_azure::azure_files::{Operation as AzureOperation, Reply, call};
use betterlite_core::api::file_content_stream::target::{
    AzureContent, CompatibleContent, ContentOperation, ContentReply, SuppliedSdk,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::{
    ClientArgs, FilesApi, FilesClient, get_openai_client, select_client,
};
use secrecy::SecretString;

use super::fixtures::azure::open;
use super::wire::{ContentWireError, deadline};

// the key and base path a case supplies with its own OpenAI client
const SUPPLIED_KEY: &str = "sk-supplied";
const SUPPLIED_SUFFIX: &str = "supplied";

// the OpenAI files client the compatible branch shares; an SDK client the caller supplied stays its own
pub(super) async fn compatible_answer(
    openai: &FilesApi<ProxiedClient>,
    content: &CompatibleContent,
    operation: ContentOperation<'_>,
) -> Result<ContentReply, ContentWireError> {
    let client = compatible_client(content)?;
    match operation {
        ContentOperation::Buffered { request } => Ok(ContentReply::Buffered(Box::new(
            openai.file_content(&client, request).await?,
        ))),
        ContentOperation::Streamed {
            request,
            chunk_size,
        } => Ok(ContentReply::Streamed(Box::new(
            openai
                .file_content_streaming(&client, request, chunk_size)
                .await?,
        ))),
    }
}

fn compatible_client(content: &CompatibleContent) -> Result<FilesClient, FilesError> {
    let (api_key, api_base, organization) = compatible_credentials(content);
    let args = ClientArgs {
        api_key,
        api_base: Some(api_base),
        timeout: deadline(content.mode),
        max_retries: content
            .max_retries
            .and_then(|retries| u32::try_from(retries).ok()),
        organization,
    };
    select_client(
        get_openai_client(args, None, content.mode.is_async)?,
        content.mode.is_async,
    )
}

// a supplied OpenAI client sends with its own base and key, as the case builds it, and the call's are unused
fn compatible_credentials(
    content: &CompatibleContent,
) -> (Option<SecretString>, String, Option<String>) {
    match content.client {
        Some(SuppliedSdk::OpenAi) => (
            Some(SecretString::from(SUPPLIED_KEY)),
            format!("{}/{SUPPLIED_SUFFIX}", content.api_base),
            None,
        ),
        Some(SuppliedSdk::Azure) | None => (
            content.api_key.clone(),
            content.api_base.clone(),
            content.organization.clone(),
        ),
    }
}

// the azure SDK call a buffered content request asks for, over the mock
pub(super) async fn azure_answer(
    client: &ProxiedClient,
    environment: &BTreeMap<String, String>,
    content: &AzureContent,
    operation: ContentOperation<'_>,
) -> Result<ContentReply, ContentWireError> {
    let opened = open(content, environment)?;
    // azure is outside the streaming set, so file_content_streaming never selects it
    let ContentOperation::Buffered { request } = operation else {
        return Err(ContentWireError::Reply);
    };
    match call(&opened, client, AzureOperation::Content(request)).await? {
        Reply::Content(binary) => Ok(ContentReply::Buffered(Box::new(binary))),
        Reply::Object(_) | Reply::Page(_) | Reply::Deleted(_) => Err(ContentWireError::Reply),
    }
}
