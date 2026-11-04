use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_tungstenite::accept_hdr_async_with_config;
use tokio_tungstenite::tungstenite::{
    Error,
    handshake::server::{Callback, ErrorResponse, Request, Response},
    http::{HeaderValue, StatusCode},
};

use super::{
    DriverError,
    socket::{Peer, config},
};
use crate::websocket_oracle::{Close, Direction, Frame, WebSocketCase};

pub(crate) struct ProviderHandle {
    pub(crate) address: SocketAddr,
    tasks: JoinSet<Result<ProviderObservation, DriverError>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderObservation {
    pub(crate) connection_count: usize,
    pub(crate) request_uri: String,
    pub(crate) frames: Vec<Frame>,
    pub(crate) close: Option<Close>,
}

pub(crate) async fn start(case: WebSocketCase) -> Result<ProviderHandle, DriverError> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        timeout(
            Duration::from_secs(case.execution_seconds),
            serve(listener, &case),
        )
        .await?
    });
    Ok(ProviderHandle { address, tasks })
}

impl ProviderHandle {
    pub(crate) async fn finish(
        mut self,
        deadline: Duration,
    ) -> Result<ProviderObservation, DriverError> {
        let result = timeout(deadline, self.tasks.join_next()).await;
        self.tasks.shutdown().await;
        result?
            .ok_or_else(|| DriverError::Transport("provider task is missing".to_owned()))?
            .map_err(DriverError::from)?
    }
    pub(crate) async fn cancel(mut self) -> Result<(), DriverError> {
        self.tasks.abort_all();
        while let Some(result) = self.tasks.join_next().await {
            match result {
                Ok(outcome) => {
                    outcome?;
                }
                Err(error) if error.is_cancelled() => {}
                Err(source) => return Err(DriverError::from(source)),
            }
        }
        Ok(())
    }
}

struct Handshake<'input> {
    uri: &'input mut String,
    case: &'input WebSocketCase,
}

impl Callback for Handshake<'_> {
    fn on_request(
        self,
        request: &Request,
        mut response: Response,
    ) -> Result<Response, ErrorResponse> {
        *self.uri = request.uri().to_string();
        if let Some(status) = self.case.upstream_status
            && status != 101
        {
            let mut refusal = ErrorResponse::new(Some("upstream refused".to_owned()));
            match StatusCode::from_u16(status) {
                Ok(code) => *refusal.status_mut() = code,
                Err(source) => {
                    *refusal.body_mut() = Some(source.to_string());
                    *refusal.status_mut() = StatusCode::BAD_REQUEST;
                }
            }
            return Err(refusal);
        }
        if let Some(protocol) = &self.case.negotiated_subprotocol
            && request.headers().contains_key("sec-websocket-protocol")
        {
            let value = HeaderValue::from_str(protocol)
                .map_err(|source| ErrorResponse::new(Some(source.to_string())))?;
            response
                .headers_mut()
                .insert("sec-websocket-protocol", value);
        }
        Ok(response)
    }
}

async fn serve(
    listener: TcpListener,
    case: &WebSocketCase,
) -> Result<ProviderObservation, DriverError> {
    let (tcp, _) = listener.accept().await?;
    let mut observation = ProviderObservation {
        connection_count: 1,
        request_uri: String::new(),
        frames: Vec::new(),
        close: None,
    };
    let handshake = Handshake {
        uri: &mut observation.request_uri,
        case,
    };
    match accept_hdr_async_with_config(tcp, handshake, Some(config())).await {
        Ok(stream) => {
            let mut socket = Peer {
                stream,
                bytes: 0,
                count: 0,
            };
            (observation.frames, observation.close) =
                socket.play(case, Direction::ProviderToClient).await?;
        }
        Err(Error::Http(_refusal)) => {}
        Err(source) => return Err(DriverError::from(source)),
    }
    Ok(observation)
}
