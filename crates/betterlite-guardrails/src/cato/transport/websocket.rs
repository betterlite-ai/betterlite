use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::SinkExt as _;
use futures_util::stream::{SplitSink, SplitStream, StreamExt as _};
use reqwest::header::HeaderMap;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, aws_lc_rs, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error as RustlsError, SignatureScheme};
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
pub struct WebSocketConnector {
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
        let connector = match &self.ssl {
            None | Some(WsSsl::SystemRoots) => None,
            Some(WsSsl::Disabled) => Some(Connector::Rustls(Arc::new(disabled_config()?))),
            Some(WsSsl::CaFile(path)) => {
                Some(Connector::Rustls(Arc::new(trusted_config(path).await?)))
            }
        };
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
            writer: Box::new(TungsteniteWriter { sink: writer }),
        })
    }
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
}

#[async_trait]
impl CatoFrameWriter for TungsteniteWriter {
    async fn send_text(&mut self, text: &str) -> Result<(), CatoError> {
        self.sink
            .send(Message::Text(Utf8Bytes::from(text)))
            .await
            .map_err(|source| CatoError::WebSocket { source })
    }
}

macro_rules! verify_signature {
    ($method:ident, $verify:path) => {
        fn $method(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, RustlsError> {
            $verify(
                message,
                cert,
                dss,
                &self.provider.signature_verification_algorithms,
            )
        }
    };
}

// ssl_verify=False verifies nothing, but still checks each handshake signature against its certificate
#[derive(Debug)]
struct NoVerification {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for NoVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    // both protocol versions check the handshake signature the same way
    verify_signature!(verify_tls12_signature, verify_tls12_signature);
    verify_signature!(verify_tls13_signature, verify_tls13_signature);

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn disabled_config() -> Result<ClientConfig, CatoError> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|source| CatoError::WsTls { source })?;
    Ok(builder
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoVerification { provider }))
        .with_no_client_auth())
}

async fn trusted_config(path: &str) -> Result<ClientConfig, CatoError> {
    let pem = tokio::fs::read(path)
        .await
        .map_err(|source| CatoError::WsCaFile { source })?;
    let roots =
        betterlite_http::tls::parse_ca_pem(&pem).map_err(|source| CatoError::Tls { source })?;
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|source| CatoError::WsTls { source })?;
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}
