use std::error::Error;
use std::future::Future;
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};

use http::Method;
use thiserror::Error;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::bounded::{CleanupError, run_bounded_cleanup};
use super::recording::{ErrorRecorder, RecordingBody, ResponseBody};

pub struct HttpResponse<Body> {
    pub status: u16,
    pub body: Body,
}

pub trait Transport: Send + Sync + 'static {
    type Request: Send + 'static;
    type Error: Error + Send + Sync + 'static;
    type Body: ResponseBody<Error = Self::Error> + 'static;

    fn method(request: &Self::Request) -> &Method;

    fn send(
        &self,
        request: Self::Request,
        stream: bool,
    ) -> impl Future<Output = Result<HttpResponse<Self::Body>, Self::Error>> + Send;

    fn status_error(&self, status: u16) -> Self::Error;
}

#[derive(Clone, Debug)]
pub struct CleanupScope {
    pub shield: bool,
    pub deadline: Instant,
    pub caller_cancelled: CancellationToken,
}

#[derive(Debug, Error)]
pub enum ClientError<Source> {
    #[error("MCP HTTP request failed")]
    Transport(#[source] Arc<Source>),
    #[error("MCP HTTP cleanup request failed")]
    Cleanup(#[source] CleanupError<Source>),
}

pub enum ClientBody<Body: ResponseBody> {
    Buffered(Vec<u8>),
    Recorded(RecordingBody<Body>),
}

impl<Body: ResponseBody> ClientBody<Body> {
    // Only streamed bodies can fail after send returns, so only they report to the recorder
    fn recorded(body: Body, shared: &Arc<Mutex<CleanupState<Body::Error>>>) -> Self {
        let state = Arc::clone(shared);
        let record_error: ErrorRecorder<Body::Error> =
            Arc::new(move |error| record_cleanup_error(&state, error));
        Self::Recorded(RecordingBody {
            inner: body,
            record_error,
        })
    }
}

impl<Body: ResponseBody> ResponseBody for ClientBody<Body> {
    type Error = Arc<Body::Error>;

    async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, Self::Error>> {
        match self {
            Self::Buffered(content) => {
                let bytes = take(content);
                (!bytes.is_empty()).then_some(Ok(bytes))
            }
            Self::Recorded(body) => body.next_chunk().await,
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        match self {
            Self::Buffered(_) => Ok(()),
            Self::Recorded(body) => body.close().await,
        }
    }

    async fn read_all(&mut self) -> Result<Vec<u8>, Self::Error> {
        match self {
            Self::Buffered(content) => Ok(take(content)),
            Self::Recorded(body) => body.read_all().await,
        }
    }
}

struct CleanupState<Source> {
    scope: Option<CleanupScope>,
    errors: Vec<Arc<Source>>,
}

// The state belongs to the client, not to a call, because one client serves many sessions
pub struct McpHttpClient<Client: Transport> {
    transport: Arc<Client>,
    tracker: TaskTracker,
    state: Arc<Mutex<CleanupState<Client::Error>>>,
}

impl<Client: Transport> McpHttpClient<Client> {
    pub fn new(transport: Arc<Client>, tracker: TaskTracker) -> Self {
        let state = CleanupState {
            scope: None,
            errors: Vec::new(),
        };
        Self {
            transport,
            tracker,
            state: Arc::new(Mutex::new(state)),
        }
    }

    // Returns the scope it replaced so a nested cleanup can restore the outer one
    pub fn enter_cleanup(&self, scope: CleanupScope) -> Option<CleanupScope> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let outer = state.scope.take();
        state.scope = Some(scope);
        outer
    }

    // Closes the scope and hands back, then forgets, every error recorded while it was open
    pub fn leave_cleanup(&self) -> Vec<Arc<Client::Error>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.scope = None;
        take(&mut state.errors)
    }

    pub async fn send(
        &self,
        request: Client::Request,
        stream: bool,
    ) -> Result<HttpResponse<ClientBody<Client::Body>>, ClientError<Client::Error>> {
        let terminal_scope = if *Client::method(&request) == Method::DELETE {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.scope.clone()
        } else {
            None
        };
        if let Some(scope) = terminal_scope {
            let operation = read_terminal(Arc::clone(&self.transport), request, stream);
            return run_bounded_cleanup(
                &self.tracker,
                &scope.caller_cancelled,
                scope.deadline,
                operation,
            )
            .await
            .map_err(ClientError::Cleanup);
        }
        self.fetch(request, stream).await.map_err(|source| {
            let error = Arc::new(source);
            record_cleanup_error(&self.state, &error);
            ClientError::Transport(error)
        })
    }

    async fn fetch(
        &self,
        request: Client::Request,
        stream: bool,
    ) -> Result<HttpResponse<ClientBody<Client::Body>>, Client::Error> {
        let is_post = *Client::method(&request) == Method::POST;
        let mut response = self.transport.send(request, stream).await?;
        // httpx reads a non-streamed body before send returns, so its read failure is a send failure
        let buffered = if stream {
            None
        } else {
            Some(response.body.read_all().await?)
        };
        if is_post && (400..600).contains(&response.status) && response.status != 404 {
            response.body.close().await?;
            return Err(self.transport.status_error(response.status));
        }
        let body = match buffered {
            Some(content) => ClientBody::Buffered(content),
            None => ClientBody::recorded(response.body, &self.state),
        };
        Ok(HttpResponse {
            status: response.status,
            body,
        })
    }
}

// A terminal DELETE reads its response so the connection is drained before the client closes
async fn read_terminal<Client>(
    transport: Arc<Client>,
    request: Client::Request,
    stream: bool,
) -> Result<HttpResponse<ClientBody<Client::Body>>, Client::Error>
where
    Client: Transport,
{
    let mut response = transport.send(request, stream).await?;
    let content = response.body.read_all().await?;
    Ok(HttpResponse {
        status: response.status,
        body: ClientBody::Buffered(content),
    })
}

// Errors are kept only while a shielded cleanup scope is open
fn record_cleanup_error<Source>(shared: &Mutex<CleanupState<Source>>, error: &Arc<Source>) {
    let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
    if state.scope.as_ref().is_some_and(|scope| scope.shield) {
        state.errors.push(Arc::clone(error));
    }
}
