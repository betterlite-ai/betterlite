use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Body;
use axum::http::{Request, Response};
use indexmap::IndexMap;
use rmcp::ServerHandler;

use session::SessionMap;

mod session;
mod stateful;
mod stateless;
pub mod wire;

pub struct StreamTransport {
    stateless: bool,
    sessions: SessionMap,
}

impl StreamTransport {
    #[must_use]
    pub fn new(stateless: bool) -> Self {
        Self {
            stateless,
            sessions: Arc::new(Mutex::new(IndexMap::new())),
        }
    }

    pub async fn handle<H>(&self, request: Request<Body>, handler: H) -> Response<Body>
    where
        H: ServerHandler,
    {
        if self.stateless {
            stateless::handle(request, handler).await
        } else {
            stateful::handle(&self.sessions, request, handler).await
        }
    }

    pub fn live_session_ids(&self) -> Vec<String> {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    pub fn has_session(&self, id: &str) -> bool {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(id)
    }

    pub fn terminate(&self, id: &str) -> bool {
        let removed = self
            .sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .shift_remove(id);
        match removed {
            Some(session) => {
                session
                    .streams
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .close_all();
                true
            }
            None => false,
        }
    }
}
