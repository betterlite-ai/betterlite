use crate::batches::decode::decode_batch_object;
use crate::batches::error::{BatchesError, SdkError};
use crate::batches::records::{BatchCreate, BatchObject, BatchQuery};
use crate::batches::sdk::{BatchesSdk, ClientConstructor, ClientKind, Connection, SdkArguments};

pub mod decode;
pub mod error;
pub mod http;
pub mod records;
pub mod sdk;

// The three calls that answer with a batch
#[derive(Debug, Clone, Copy)]
pub enum BatchOperation<'request> {
    Create(&'request BatchCreate),
    Retrieve(&'request BatchQuery),
    Cancel(&'request BatchQuery),
}

// LiteLLM's OpenAIBatchesAPI over an injected SDK client
#[derive(Debug)]
pub struct BatchesApi<Constructor> {
    pub constructor: Constructor,
}

impl<Constructor: ClientConstructor> BatchesApi<Constructor> {
    // A supplied client is used as is; otherwise one is built from the settings
    pub fn get_openai_client(
        &self,
        connection: Connection<Constructor::Client>,
    ) -> Result<Option<Constructor::Client>, SdkError> {
        let Connection {
            is_async,
            settings,
            client,
        } = connection;
        if client.is_some() {
            return Ok(client);
        }
        let arguments = SdkArguments {
            api_key: settings.api_key,
            base_url: settings.api_base,
            timeout: settings.timeout,
            max_retries: settings.max_retries,
            organization: settings.organization,
        };
        let kind = if is_async {
            ClientKind::Async
        } else {
            ClientKind::Sync
        };
        self.constructor.construct(&arguments, kind)
    }

    // Only cancel checks the sync class; the other sync calls trust the client they were given
    fn checked_client(
        &self,
        connection: Connection<Constructor::Client>,
        check_sync: bool,
    ) -> Result<Constructor::Client, BatchesError> {
        let is_async = connection.is_async;
        let client = self
            .get_openai_client(connection)?
            .ok_or(BatchesError::NotInitialized)?;
        if is_async && client.kind() != ClientKind::Async {
            return Err(BatchesError::NotAsync);
        }
        if !is_async && check_sync && client.kind() != ClientKind::Sync {
            return Err(BatchesError::NotSync);
        }
        Ok(client)
    }

    // create, retrieve and cancel decode the SDK's model_dump into a batch; only cancel checks the sync class
    pub async fn batch(
        &self,
        connection: Connection<Constructor::Client>,
        operation: BatchOperation<'_>,
    ) -> Result<BatchObject, BatchesError> {
        let client =
            self.checked_client(connection, matches!(operation, BatchOperation::Cancel(_)))?;
        let body = match operation {
            BatchOperation::Create(request) => client.create(request).await,
            BatchOperation::Retrieve(request) => client.retrieve(request).await,
            BatchOperation::Cancel(request) => client.cancel(request).await,
        }?;
        Ok(decode_batch_object(&body)?)
    }

    // The SDK page comes back unchanged, and a missing cursor or limit is passed on as such
    pub async fn list_batches(
        &self,
        connection: Connection<Constructor::Client>,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> Result<<Constructor::Client as BatchesSdk>::Page, BatchesError> {
        Ok(self
            .checked_client(connection, false)?
            .list(after, limit)
            .await?)
    }
}

mod tests;
