pub mod endpoint;
pub mod error;
pub mod failure;
pub mod init_params;
pub mod request;
pub mod token_response;
pub mod types;

use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use tokio::sync::Mutex;

use crate::zerobus::client::error::ZerobusClientError;
use crate::zerobus::client::failure::status_failure;
use crate::zerobus::client::request::{Endpoint, outbound_request};
use crate::zerobus::client::token_response::decode_token_response;
use crate::zerobus::client::types::{
    TOKEN_REFRESH_LEEWAY_SECONDS, ZerobusAccessToken, ZerobusConnection, ZerobusIngestFailure,
};
use crate::zerobus::row::TraceRow;

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

pub struct ZerobusIngestClient {
    pub connection: ZerobusConnection,
    pub http: ProxiedClient,
    pub clock: Clock,
    pub timeout: Duration,
    pub token: Mutex<Option<ZerobusAccessToken>>,
}

impl ZerobusIngestClient {
    pub async fn insert(&self, rows: &[TraceRow]) -> Result<(), ZerobusIngestFailure> {
        let token = self.access_token().await?;
        let body = serde_json::to_vec(rows).map_err(ZerobusClientError::Rows)?;
        let request = outbound_request(
            &self.connection,
            Endpoint::Insert {
                rows: body,
                token: &token.value,
            },
            self.timeout,
        )?;
        let response = match self.http.send_buffered(request).await {
            Ok(response) => response,
            Err(source) => {
                return Err(ZerobusIngestFailure {
                    detail: format!("insert failed: {source}"),
                    retryable: true,
                });
            }
        };
        if (200..300).contains(&response.status) {
            return Ok(());
        }
        if response.status == 401 {
            *self.token.lock().await = None;
            return Err(ZerobusIngestFailure {
                detail: "insert returned 401, token discarded".to_owned(),
                retryable: true,
            });
        }
        Err(status_failure("insert", response.status, &response.body))
    }

    pub async fn access_token(&self) -> Result<ZerobusAccessToken, ZerobusIngestFailure> {
        let mut cached = self.token.lock().await;
        if let Some(token) = cached.as_ref()
            && token.expires_at > (self.clock)().mul_add(1.0, TOKEN_REFRESH_LEEWAY_SECONDS)
        {
            return Ok(token.clone());
        }
        let minted = self.mint_token().await?;
        *cached = Some(minted.clone());
        drop(cached);
        Ok(minted)
    }

    async fn mint_token(&self) -> Result<ZerobusAccessToken, ZerobusIngestFailure> {
        let request = outbound_request(&self.connection, Endpoint::Token, self.timeout)?;
        let response = match self.http.send_buffered(request).await {
            Ok(response) => response,
            Err(source) => {
                return Err(ZerobusIngestFailure {
                    detail: format!("token request failed: {source}"),
                    retryable: true,
                });
            }
        };
        if !(200..300).contains(&response.status) {
            return Err(status_failure(
                "token request",
                response.status,
                &response.body,
            ));
        }
        decode_token_response(&response.body, (self.clock)())
    }
}
