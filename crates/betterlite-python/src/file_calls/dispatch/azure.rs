use betterlite_azure::azure_files::client::{
    ClientMaterializer, ClientSettings, Connection, Opened, SelectedClient,
};
use betterlite_azure::azure_files::error::AzureFilesError;
use betterlite_azure::azure_files::{Operation as AzureOperation, Reply as AzureReply, call};
use betterlite_azure::credentials::cache::RefreshProviderCache;
use betterlite_azure::endpoints::get_client::ClientCache;
use betterlite_azure::endpoints::projection::{
    ClientIdentity, ClientKind, ClientOutcome, LiteLlmParams, SessionKind,
};
use betterlite_core::api::assistant_read_delete::sdk_dispatch::NoSources;
use betterlite_core::api::file_provider_read::target::{
    AzureRead, FileListing, ReadAnswer, ReadOperation,
};
use betterlite_openai::files::records::{FileObject, FilePage};

use super::DispatchError;
use super::FileCallDispatch;
use super::clients::{read_litellm_params, settings};

// the azure call both operations share, with the timeout and max_retries both methods take
pub(super) async fn azure_answer(
    dispatch: &FileCallDispatch,
    read: &AzureRead,
    operation: ReadOperation<'_>,
) -> Result<ReadAnswer, DispatchError> {
    let opened = open_azure(
        dispatch,
        settings(
            read.api_key.clone(),
            read.api_base.clone(),
            read.api_version.clone(),
        ),
        read.mode.is_async,
        read_litellm_params(read),
    )?;
    let asked = match operation {
        ReadOperation::Retrieve { file_id } => AzureOperation::Retrieve(file_id),
        ReadOperation::List { purpose } => AzureOperation::List(purpose),
    };
    match call(&opened, &dispatch.client, asked).await? {
        AzureReply::Object(object) => file_answer(operation, object),
        AzureReply::Page(page) => page_answer(operation, page),
        AzureReply::Content(_) | AzureReply::Deleted(_) => Err(DispatchError::Reply),
    }
}

fn file_answer(
    operation: ReadOperation<'_>,
    object: FileObject,
) -> Result<ReadAnswer, DispatchError> {
    match operation {
        ReadOperation::Retrieve { .. } => Ok(ReadAnswer::File(Box::new(object))),
        ReadOperation::List { .. } => Err(DispatchError::Reply),
    }
}

fn page_answer(operation: ReadOperation<'_>, page: FilePage) -> Result<ReadAnswer, DispatchError> {
    match operation {
        ReadOperation::List { .. } => Ok(ReadAnswer::Listing(FileListing::Page(Box::new(page)))),
        ReadOperation::Retrieve { .. } => Err(DispatchError::Reply),
    }
}

pub(super) fn open_azure(
    dispatch: &FileCallDispatch,
    settings: ClientSettings,
    is_async: bool,
    params: LiteLlmParams,
) -> Result<Opened, AzureFilesError> {
    let sources = NoSources;
    let materializer = NoMaterializer;
    let mut cache = NoCache;
    let mut refresh = RefreshProviderCache::new(&sources);
    Connection {
        settings,
        params,
        provider: None,
        supplied: None,
        is_async,
        endpoints: &dispatch.endpoints,
        environment: &dispatch.environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer: &materializer,
    }
    .open()
}

// nothing is supplied or cached, so the azure handler always builds its client from its own arguments
struct NoCache;

impl ClientCache for NoCache {
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        None
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}

struct NoMaterializer;

impl ClientMaterializer for NoMaterializer {
    fn materialize(
        &self,
        _outcome: &ClientOutcome<'_>,
    ) -> Result<Option<SelectedClient>, AzureFilesError> {
        Ok(None)
    }
}
