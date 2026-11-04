use std::net::SocketAddr;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
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
    Clock, DriverError,
    socket::{Peer, config},
};
use crate::websocket_oracle::{
    AuthObservation, Close, Direction, Frame, OracleLimits, RouteObservation, WebSocketCase,
};

pub(crate) struct ProviderHandle {
    pub(crate) address: SocketAddr,
    tasks: JoinSet<Result<ProviderObservation, DriverError>>,
    connections: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderObservation {
    pub(crate) connection_count: usize,
    pub(crate) request_uri: String,
    pub(crate) frames: Vec<Frame>,
    pub(crate) close: Option<Close>,
    pub(crate) route: RouteObservation,
    pub(crate) auth: AuthObservation,
}

pub(crate) async fn start<C>(
    case: WebSocketCase,
    limits: OracleLimits,
    clock: &C,
) -> Result<ProviderHandle, DriverError>
where
    C: Clock + Clone + 'static,
{
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let mut tasks = JoinSet::new();
    let connections = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&connections);
    let owned_clock = clock.clone();
    tasks.spawn(async move {
        owned_clock
            .timeout(
                Duration::from_secs(case.execution_seconds),
                serve(listener, &case, limits, &counter),
            )
            .await?
    });
    Ok(ProviderHandle {
        address,
        tasks,
        connections,
    })
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
    pub(crate) async fn cancel(mut self) -> Result<usize, DriverError> {
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
        Ok(self.connections.load(Ordering::Relaxed))
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
            *refusal.status_mut() = StatusCode::from_u16(status).unwrap_or_else(|source| {
                *refusal.body_mut() = Some(source.to_string());
                StatusCode::BAD_REQUEST
            });
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
    limits: OracleLimits,
    connections: &AtomicUsize,
) -> Result<ProviderObservation, DriverError> {
    let (tcp, _) = listener.accept().await?;
    connections.fetch_add(1, Ordering::Relaxed);
    let mut observation = ProviderObservation {
        connection_count: 1,
        request_uri: String::new(),
        frames: Vec::new(),
        close: None,
        route: RouteObservation {
            model: case.route_model.clone(),
            guardrails: case.route_guardrails.clone(),
        },
        auth: AuthObservation {
            detail: case.authentication_detail.clone(),
            attempted_close_code: case.attempted_close_code,
        },
    };
    let handshake = Handshake {
        uri: &mut observation.request_uri,
        case,
    };
    match accept_hdr_async_with_config(tcp, handshake, Some(config(&limits))).await {
        Ok(stream) => {
            let mut socket = Peer {
                stream,
                bytes: 0,
                count: 0,
                limits,
            };
            (observation.frames, observation.close) =
                socket.play(case, Direction::ProviderToClient).await?;
        }
        Err(Error::Http(_refusal)) => {}
        Err(source) => return Err(DriverError::from(source)),
    }
    Ok(observation)
}
