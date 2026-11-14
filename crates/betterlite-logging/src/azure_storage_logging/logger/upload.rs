use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use secrecy::SecretString;

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
// the SDK's flush_data adds these two to LiteLLM's REST flush
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
        let auth = StorageAuthorization::Bearer(&token);
        let dumped = safe_dumps(payload, DEFAULT_MAX_RECURSE_DEPTH, None)
            .map_err(AzureStorageError::Serialize)?;
        let json_payload = format!("{dumped}\n");
        let now = self.deps.clock.now_unix();
        let file_name = adls_safe_file_name(Some(payload.id.as_str()), self.deps.ids.as_ref());
        let endpoint = format!(
            "https://{}.dfs.{}",
            self.settings.account_name, self.settings.endpoint_suffix
        );
        let base_url = storage_url(
            &endpoint,
            &[self.settings.file_system.as_str(), file_name.as_str()],
        );
        let _created = self
            .execute(create_file_request(&base_url, &auth, now)?, "create")
            .await?;
        let _appended = self
            .execute(
                append_request(&base_url, &auth, now, json_payload.as_bytes())?,
                "append",
            )
            .await?;
        let _flushed = self
            .execute(
                flush_data_request(&base_url, &auth, now, json_payload.len(), "")?,
                "flush",
            )
            .await?;
        Ok(())
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
        let endpoint = format!(
            "https://{}.dfs.{}",
            self.settings.account_name, self.settings.endpoint_suffix
        );
        let directory_url = storage_url(
            &endpoint,
            &[self.settings.file_system.as_str(), directory.as_str()],
        );
        // the SDK's directory exists() is a HEAD on the blob endpoint
        let exists_url = storage_url(
            &self.settings.blob_endpoint(),
            &[self.settings.file_system.as_str(), directory.as_str()],
        );
        let exists = self
            .execute(directory_exists_request(&exists_url, &auth, now)?, "exists")
            .await?;
        if exists.status == NOT_FOUND {
            let _made = self
                .execute(
                    create_directory_request(&directory_url, &auth, now)?,
                    "create_directory",
                )
                .await?;
        }
        let file_name = adls_safe_file_name(Some(payload.id.as_str()), self.deps.ids.as_ref());
        let base_url = storage_url(&directory_url, &[file_name.as_str()]);
        let content = safe_dumps(payload, DEFAULT_MAX_RECURSE_DEPTH, None)
            .map_err(AzureStorageError::Serialize)?
            .into_bytes();
        let position = content.len();
        let _created = self
            .execute(create_file_request(&base_url, &auth, now)?, "create")
            .await?;
        let _appended = self
            .execute(append_request(&base_url, &auth, now, &content)?, "append")
            .await?;
        let _flushed = self
            .execute(
                flush_data_request(&base_url, &auth, now, position, SDK_FLUSH_QUERY)?,
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
        Ok(provider)
    }

    pub async fn set_valid_azure_ad_token(&self) -> Result<SecretString, TokenError> {
        if self.settings.tenant_id.is_none()
            && self.settings.client_id.is_none()
            && self.settings.client_secret.is_none()
        {
            let provider = self.chain_provider()?;
            // the chain walk probes IMDS and may run a CLI, so it runs off the async worker
            let token = tokio::task::spawn_blocking(move || provider.token())
                .await
                .map_err(TokenError::Worker)??;
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.token.token = Some(token.clone());
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
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.token.store(token.clone(), now);
        Ok(token)
    }

    // the SDK client is rebuilt once its TTL passes, which is LiteLLM's connection-pool reset
    fn service_client(&self, now: Duration) -> Result<CachedClient, AzureStorageError> {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(cached) = state.client.take() {
                if cached.expires_at > now {
                    let client = cached.clone();
                    state.client = Some(cached);
                    return Ok(client);
                }
                let _closed = self.closed_clients.fetch_add(1, Ordering::SeqCst);
            }
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
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.client = Some(created.clone());
        Ok(created)
    }
}
