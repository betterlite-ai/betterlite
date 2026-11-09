use std::collections::VecDeque;
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};

use crate::secret_read_dispatch::http_transport::{
    ExchangeFuture, HttpError, HttpExchange, HttpRequest, HttpResponse,
};

pub(super) struct ScriptedExchange {
    replies: Mutex<VecDeque<Result<HttpResponse, HttpError>>>,
    requests: Mutex<Vec<HttpRequest>>,
}

impl ScriptedExchange {
    pub(super) fn requests(&self) -> Vec<HttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| {
                Ok(HttpResponse {
                    status: 200,
                    body: Vec::new(),
                })
            })
    }
}

impl HttpExchange for ScriptedExchange {
    fn send(&self, request: HttpRequest) -> ExchangeFuture<'_> {
        Box::pin(ready(self.answer(request)))
    }

    fn send_blocking(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        self.answer(request)
    }
}

pub(super) fn scripted(
    replies: Vec<Result<HttpResponse, HttpError>>,
) -> (Arc<dyn HttpExchange>, Arc<ScriptedExchange>) {
    let scripted = Arc::new(ScriptedExchange {
        replies: Mutex::new(VecDeque::from(replies)),
        requests: Mutex::new(Vec::new()),
    });
    let concrete = Arc::clone(&scripted);
    let exchange: Arc<dyn HttpExchange> = concrete;
    (exchange, scripted)
}

pub(super) fn reply(status: u16, body: &[u8]) -> HttpResponse {
    HttpResponse {
        status,
        body: body.to_vec(),
    }
}

pub(super) fn header<'request>(
    request: &'request HttpRequest,
    name: &str,
) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(header, _)| *header == name)
        .map(|(_, value)| value.expose())
}
