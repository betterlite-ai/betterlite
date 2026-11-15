use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::tls::Verification;
use betterlite_http::tls::{TlsError, websocket_client_config};
use futures_util::SinkExt as _;
use futures_util::stream::{SplitSink, SplitStream, StreamExt as _};
use reqwest::header::HeaderMap;
use tokio::net::TcpStream;
use tokio::time;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
};

use crate::cato::error::CatoError;
use crate::cato::{CatoGuardrail, WsSsl};

use super::stream::{
    CatoFrameReader, CatoFrameWriter, CatoSocket, CatoSocketConnector, MAX_FRAME_BYTES,
};

type SocketStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

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
        let mut request = url
            .into_client_request()
            .map_err(|source| CatoError::WebSocket { source })?;
        request.headers_mut().extend(headers.clone());
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME_BYTES))
            .max_frame_size(Some(MAX_FRAME_BYTES));
        let chosen = match &self.ssl {
            None | Some(WsSsl::SystemRoots) => None,
            Some(WsSsl::Disabled) => Some(Verification::Disabled),
            Some(WsSsl::CaFile(path)) => Some(Verification::CaFile(path.clone())),
        };
        let connector = chosen.as_ref().map(rustls_connector).transpose()?;
        let opening = connect_async_tls_with_config(request, Some(config), false, connector);
        let (socket, _answer) = time::timeout(open, opening)
            .await
            .map_err(|_elapsed| CatoError::ConnectTimeout)?
            .map_err(|source| CatoError::WebSocket { source })?;
        let (writer, reader) = socket.split();
        Ok(CatoSocket {
            reader: Box::new(TungsteniteReader {
                stream: reader,
                read_timeout: read,
            }),
            writer: Box::new(TungsteniteWriter {
                sink: writer,
                write_timeout: read,
            }),
        })
    }
}

// the rustls connector for a verification, with the CA file failures named as the websocket's own
fn rustls_connector(verification: &Verification) -> Result<Connector, CatoError> {
    let tls_config = websocket_client_config(verification).map_err(|error| match error {
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
    })?;
    Ok(Connector::Rustls(Arc::new(tls_config)))
}

struct TungsteniteReader {
    stream: SplitStream<SocketStream>,
    read_timeout: Duration,
}

#[async_trait]
impl CatoFrameReader for TungsteniteReader {
    async fn recv_message(&mut self) -> Result<Option<Vec<u8>>, CatoError> {
        loop {
            let next = time::timeout(self.read_timeout, self.stream.next())
                .await
                .map_err(|_elapsed| CatoError::StreamTimeout)?;
            match next {
                Some(Ok(Message::Text(text))) => {
                    return Ok(Some(text.as_str().as_bytes().to_vec()));
                }
                Some(Ok(Message::Binary(bytes))) => return Ok(Some(bytes.to_vec())),
                Some(Ok(Message::Close(_))) | None => return Ok(None),
                // tungstenite answers pings itself and a bare frame carries no message
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                Some(Err(source)) => return Err(CatoError::WebSocket { source }),
            }
        }
    }
}

struct TungsteniteWriter {
    sink: SplitSink<SocketStream, Message>,
    // a server that keeps its socket open but stops reading must not hold the forwarding task forever
    write_timeout: Duration,
}

#[async_trait]
impl CatoFrameWriter for TungsteniteWriter {
    async fn send_text(&mut self, text: &str) -> Result<(), CatoError> {
        time::timeout(
            self.write_timeout,
            self.sink.send(Message::Text(Utf8Bytes::from(text))),
        )
        .await
        .map_err(|_elapsed| CatoError::StreamTimeout)?
        .map_err(|source| CatoError::WebSocket { source })
    }
}
