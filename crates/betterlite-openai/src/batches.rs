use crate::batches::decode::decode_batch_object;
use crate::batches::error::BatchesError;
use crate::batches::records::{BatchCreate, BatchObject, BatchQuery};
use crate::batches::sdk::{BatchesSdk, ClientConstructor, ClientKind, Connection, SdkArguments};

pub mod decode;
pub mod error;
pub mod http;
pub mod records;
pub mod sdk;

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
    ) -> Option<Constructor::Client> {
        let Connection {
            is_async,
            settings,
            client,
        } = connection;
        if client.is_some() {
            return client;
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
            .get_openai_client(connection)
            .ok_or(BatchesError::NotInitialized)?;
        if is_async && client.kind() != ClientKind::Async {
            return Err(BatchesError::NotAsync);
        }
        if !is_async && check_sync && client.kind() != ClientKind::Sync {
            return Err(BatchesError::NotSync);
        }
        Ok(client)
    }

    pub async fn create_batch(
        &self,
        connection: Connection<Constructor::Client>,
        request: &BatchCreate,
    ) -> Result<BatchObject, BatchesError> {
        let client = self.checked_client(connection, false)?;
        let body = client.create(request).await?;
        Ok(decode_batch_object(&body)?)
    }

    pub async fn retrieve_batch(
        &self,
        connection: Connection<Constructor::Client>,
        request: &BatchQuery,
    ) -> Result<BatchObject, BatchesError> {
        let client = self.checked_client(connection, false)?;
        let body = client.retrieve(request).await?;
        Ok(decode_batch_object(&body)?)
    }

    pub async fn cancel_batch(
        &self,
        connection: Connection<Constructor::Client>,
        request: &BatchQuery,
    ) -> Result<BatchObject, BatchesError> {
        let client = self.checked_client(connection, true)?;
        let body = client.cancel(request).await?;
        Ok(decode_batch_object(&body)?)
    }

    // The SDK page comes back unchanged, and a missing cursor or limit is passed on as such
    pub async fn list_batches(
        &self,
        connection: Connection<Constructor::Client>,
        after: Option<&str>,
        limit: Option<u32>,
    ) -> Result<<Constructor::Client as BatchesSdk>::Page, BatchesError> {
        Ok(self
            .checked_client(connection, false)?
            .list(after, limit)
            .await?)
    }
}

mod tests;
