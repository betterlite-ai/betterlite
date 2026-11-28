use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::tls::Verification;
use betterlite_http::tls::{TlsError, websocket_client_config};
use betterlite_http::websocket_client::connect;
use betterlite_http::websocket_client::connection::{
    WebSocketConnection, WebSocketReceive, WebSocketReceiver, WebSocketSender,
};
use betterlite_http::websocket_client::error::WebSocketClientError;
use betterlite_http::websocket_client::request::WebSocketRequest;
use reqwest::header::HeaderMap;
use rustls::ClientConfig;
use tokio::time;
use url::Url;

use crate::cato::error::CatoError;
use crate::cato::{CatoGuardrail, WsSsl};

use super::stream::{
    CatoFrameReader, CatoFrameWriter, CatoSocket, CatoSocketConnector, MAX_FRAME_BYTES,
};

// LiteLLM's websockets connect call: no ssl argument uses the system roots, and ssl_verify chooses the others
#[derive(Debug, Clone)]
pub(crate) struct WebSocketConnector {
    ssl: Option<WsSsl>,
}

impl From<&CatoGuardrail> for WebSocketConnector {
    fn from(guardrail: &CatoGuardrail) -> Self {
        Self {
            ssl: guardrail.ws_ssl.clone(),
        }
    }
}

#[async_trait]
impl CatoSocketConnector for WebSocketConnector {
    async fn connect(
        &self,
        url: &str,
        headers: &HeaderMap,
        open: Duration,
        read: Duration,
    ) -> Result<CatoSocket, CatoError> {
        let url = Url::parse(url).map_err(|_error| CatoError::WebSocket {
            source: WebSocketClientError::Url,
        })?;
        let mut pairs = Vec::with_capacity(headers.len());
        for (name, value) in headers {
            pairs.push((name.clone(), value.clone()));
        }
        let chosen = match &self.ssl {
            None | Some(WsSsl::SystemRoots) => None,
            Some(WsSsl::Disabled) => Some(Verification::Disabled),
            Some(WsSsl::CaFile(path)) => Some(Verification::CaFile(path.clone())),
        };
        let tls = chosen.as_ref().map(tls_config).transpose()?;
        let connection = connect(WebSocketRequest {
            url,
            headers: &pairs,
            tls: tls.map(Arc::new),
            max_message_size: MAX_FRAME_BYTES,
            open_timeout: open,
            keepalive: None,
            proxy: None,
        })
        .await
        .map_err(connect_error)?;
        let WebSocketConnection { sender, receiver } = connection;
        Ok(CatoSocket {
            reader: Box::new(ClientFrameReader {
                receiver,
                read_timeout: read,
            }),
            writer: Box::new(ClientFrameWriter {
                sender,
                write_timeout: read,
            }),
        })
    }
}

fn connect_error(error: WebSocketClientError) -> CatoError {
    match error {
        WebSocketClientError::Timeout => CatoError::ConnectTimeout,
        other => CatoError::WebSocket { source: other },
    }
}

// the rustls client config for a verification, with the CA file failures named as the websocket's own
fn tls_config(verification: &Verification) -> Result<ClientConfig, CatoError> {
    websocket_client_config(verification).map_err(|error| match error {
        TlsError::ReadCa(source) => CatoError::WsCaFile { source },
        TlsError::RustlsConfig(source) => CatoError::WsTls { source },
        other @ (TlsError::Http(_)
        | TlsError::Policy(_)
        | TlsError::CaLength(_)
        | TlsError::CaTooLarge
        | TlsError::EmptyCa
        | TlsError::UnsupportedCurve(_)
        | TlsError::InvalidPem(_)
        | TlsError::InvalidDer(_)
        | TlsError::Signatures(_)
        | TlsError::UserAgent(_)) => CatoError::Tls { source: other },
    })
}

struct ClientFrameReader {
    receiver: WebSocketReceiver,
    read_timeout: Duration,
}

#[async_trait]
impl CatoFrameReader for ClientFrameReader {
    async fn recv_message(&mut self) -> Result<Option<Vec<u8>>, CatoError> {
        let next = time::timeout(self.read_timeout, self.receiver.recv())
            .await
            .map_err(|_elapsed| CatoError::StreamTimeout)?
            .map_err(|source| CatoError::WebSocket { source })?;
        match next {
            WebSocketReceive::Text(text) => Ok(Some(text.into_bytes())),
            WebSocketReceive::Binary(bytes) => Ok(Some(bytes.to_vec())),
            WebSocketReceive::Close(_) | WebSocketReceive::End => Ok(None),
        }
    }
}

struct ClientFrameWriter {
    sender: WebSocketSender,
    // a server that keeps its socket open but stops reading must not hold the forwarding task forever
    write_timeout: Duration,
}

#[async_trait]
impl CatoFrameWriter for ClientFrameWriter {
    async fn send_text(&mut self, text: &str) -> Result<(), CatoError> {
        time::timeout(self.write_timeout, self.sender.send_text(text))
            .await
            .map_err(|_elapsed| CatoError::StreamTimeout)?
            .map_err(|source| CatoError::WebSocket { source })
    }
}
