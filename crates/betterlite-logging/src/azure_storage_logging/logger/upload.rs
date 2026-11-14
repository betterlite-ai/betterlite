use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use secrecy::SecretString;
use tokio::runtime::Handle;

use crate::fields::payload::StandardLoggingPayload;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::{
    AzureBlobStorageLogger, AzureStorageError, CachedClient, SERVICE_CLIENT_TTL, dated_directory,
};
use crate::azure_storage_logging::filename::{adls_safe_file_name, storage_url};
use crate::azure_storage_logging::http_ops::{
    StorageAuthorization, append_request, create_directory_request, create_file_request,
    directory_exists_request, flush_data_request,
};
use crate::azure_storage_logging::token::{
    ChainTokenProvider, TokenError, client_secret_request, credentials, parse_token_response,
};
use crate::azure_storage_logging::transport::{StorageRequest, StorageResponse};

const FIRST_ERROR_STATUS: u16 = 400;
const NOT_FOUND: u16 = 404;
const SDK_FLUSH_QUERY: &str = "&retainUncommittedData=false&close=false";

impl AzureBlobStorageLogger {
    pub async fn upload_payload(
        &self,
        payload: &StandardLoggingPayload,
    ) -> Result<(), AzureStorageError> {
        if self.settings.account_key.is_some() {
            self.upload_with_account_key(payload).await
        } else {
            self.upload_with_token(payload).await
        }
    }

    async fn upload_with_token(
        &self,
        payload: &StandardLoggingPayload,
    ) -> Result<(), AzureStorageError> {
        let token = self.set_valid_azure_ad_token().await?;
        let dumped = safe_dumps(payload, DEFAULT_MAX_RECURSE_DEPTH, None)
            .map_err(AzureStorageError::Serialize)?;
        let file_name = adls_safe_file_name(Some(payload.id.as_str()), self.deps.ids.as_ref());
        let base_url = storage_url(
            &self.settings.dfs_endpoint(),
            &[self.settings.file_system.as_str(), file_name.as_str()],
        );
        let auth = StorageAuthorization::Bearer(&token);
        self.write_file(&base_url, &auth, format!("{dumped}\n").as_bytes())
            .await
    }

    async fn upload_with_account_key(
        &self,
        payload: &StandardLoggingPayload,
    ) -> Result<(), AzureStorageError> {
        let now = self.deps.clock.now_unix();
        let client = self.service_client(now)?;
        let auth = StorageAuthorization::SharedKey {
            account: &client.account,
            key: &client.key,
        };
        let directory = dated_directory(now).ok_or(AzureStorageError::Date)?;
        let segments = [self.settings.file_system.as_str(), directory.as_str()];
        let directory_url = storage_url(&self.settings.dfs_endpoint(), &segments);
        let exists_url = storage_url(&self.settings.blob_endpoint(), &segments);
        self.ensure_directory(&exists_url, &directory_url, &auth)
            .await?;
        let file_name = adls_safe_file_name(Some(payload.id.as_str()), self.deps.ids.as_ref());
        let content = safe_dumps(payload, DEFAULT_MAX_RECURSE_DEPTH, None)
            .map_err(AzureStorageError::Serialize)?;
        let base_url = storage_url(&directory_url, &[file_name.as_str()]);
        self.write_file(&base_url, &auth, content.as_bytes()).await
    }

    // the SDK's directory exists() is a HEAD on the blob endpoint, and a 404 there means create it
    async fn ensure_directory(
        &self,
        exists_url: &str,
        directory_url: &str,
        auth: &StorageAuthorization<'_>,
    ) -> Result<(), AzureStorageError> {
        let now = self.deps.clock.now_unix();
        let exists = self
            .deps
            .transport
            .send(directory_exists_request(exists_url, auth, now)?)
            .await?;
        if exists.status == NOT_FOUND {
            let _made = self
                .execute(
                    create_directory_request(directory_url, auth, now)?,
                    "create_directory",
                )
                .await?;
            return Ok(());
        }
        if exists.status >= FIRST_ERROR_STATUS {
            return Err(AzureStorageError::Status {
                operation: "exists",
                status: exists.status,
            });
        }
        Ok(())
    }

    // create, append, flush; the SDK's flush adds retainUncommittedData and close to LiteLLM's REST flush
    async fn write_file(
        &self,
        base_url: &str,
        auth: &StorageAuthorization<'_>,
        content: &[u8],
    ) -> Result<(), AzureStorageError> {
        let now = self.deps.clock.now_unix();
        let flush_query = match auth {
            StorageAuthorization::Bearer(_) => "",
            StorageAuthorization::SharedKey { .. } => SDK_FLUSH_QUERY,
        };
        let _created = self
            .execute(create_file_request(base_url, auth, now)?, "create")
            .await?;
        let _appended = self
            .execute(append_request(base_url, auth, now, content)?, "append")
            .await?;
        let _flushed = self
            .execute(
                flush_data_request(base_url, auth, now, content.len(), flush_query)?,
                "flush",
            )
            .await?;
        Ok(())
    }

    async fn execute(
        &self,
        request: StorageRequest,
        operation: &'static str,
    ) -> Result<StorageResponse, AzureStorageError> {
        let response = self.deps.transport.send(request).await?;
        if response.status >= FIRST_ERROR_STATUS {
            return Err(AzureStorageError::Status {
                operation,
                status: response.status,
            });
        }
        Ok(response)
    }

    // LiteLLM caches the credential-chain token provider in its factory, so construction happens once
    fn chain_provider(&self) -> Result<Arc<dyn ChainTokenProvider>, TokenError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(provider) = &state.chain {
            return Ok(Arc::clone(provider));
        }
        let provider = self.deps.chain_factory.provider()?;
        state.chain = Some(Arc::clone(&provider));
        drop(state);
        Ok(provider)
    }

    pub async fn set_valid_azure_ad_token(&self) -> Result<SecretString, TokenError> {
        if self.settings.tenant_id.is_none()
            && self.settings.client_id.is_none()
            && self.settings.client_secret.is_none()
        {
            let provider = self.chain_provider()?;
            // the chain walk probes IMDS and may run a CLI, so it runs off the async worker
            let token = Handle::current()
                .spawn_blocking(move || provider.token())
                .await
                .map_err(TokenError::Worker)??;
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .token
                .token = Some(token.clone());
            return Ok(token);
        }
        let now = self.deps.clock.now_unix();
        let cached = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if state.token.is_expired(now) {
                None
            } else {
                state.token.token.clone()
            }
        };
        if let Some(token) = cached {
            return Ok(token);
        }
        let credentials = credentials(&self.settings)?;
        let response = self
            .deps
            .transport
            .send(client_secret_request(&credentials))
            .await?;
        let token = parse_token_response(&response)?;
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .token
            .store(token.clone(), now);
        Ok(token)
    }

    // the SDK client is rebuilt once its TTL passes, which is LiteLLM's connection-pool reset
    fn service_client(&self, now: Duration) -> Result<CachedClient, AzureStorageError> {
        let cached = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .client
            .clone();
        if let Some(client) = cached {
            if client.expires_at > now {
                return Ok(client);
            }
            let _closed = self.closed_clients.fetch_add(1, Ordering::SeqCst);
        }
        let key = self
            .settings
            .account_key
            .clone()
            .ok_or(AzureStorageError::MissingAccountKey)?;
        let created = CachedClient {
            account: self.settings.account_name.clone(),
            key,
            expires_at: now.saturating_add(SERVICE_CLIENT_TTL),
        };
        let _created = self.created_clients.fetch_add(1, Ordering::SeqCst);
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .client = Some(created.clone());
        Ok(created)
    }
}
