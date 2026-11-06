mod bounded;
mod client;
mod failure;
mod recording;

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::Method;
use thiserror::Error;
use tokio::time::{Instant, sleep};

use super::super::http_client::{HttpResponse, Transport};
use super::super::recording::ResponseBody;

#[derive(Debug, Error, PartialEq, Eq)]
enum FakeError {
    #[error("boom: {0}")]
    Boom(&'static str),
    #[error("status {0}")]
    Status(u16),
}

struct FakeBody {
    chunks: VecDeque<Result<Vec<u8>, FakeError>>,
    close_error: Option<FakeError>,
    closed: Arc<AtomicBool>,
    read: Arc<AtomicBool>,
}

impl FakeBody {
    fn new(chunks: Vec<Result<Vec<u8>, FakeError>>) -> Self {
        Self {
            chunks: VecDeque::from(chunks),
            close_error: None,
            closed: Arc::new(AtomicBool::new(false)),
            read: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ResponseBody for FakeBody {
    type Error = FakeError;

    fn next_chunk(&mut self) -> impl Future<Output = Option<Result<Vec<u8>, FakeError>>> + Send {
        ready(self.chunks.pop_front())
    }

    fn close(&mut self) -> impl Future<Output = Result<(), FakeError>> + Send {
        self.closed.store(true, Ordering::SeqCst);
        ready(self.close_error.take().map_or(Ok(()), Err))
    }

    fn read_all(&mut self) -> impl Future<Output = Result<Vec<u8>, FakeError>> + Send {
        self.read.store(true, Ordering::SeqCst);
        let mut content = Vec::new();
        while let Some(chunk) = self.chunks.pop_front() {
            match chunk {
                Ok(bytes) => content.extend(bytes),
                Err(error) => return ready(Err(error)),
            }
        }
        ready(Ok(content))
    }
}

struct Reply {
    delay: Duration,
    outcome: Result<HttpResponse<FakeBody>, FakeError>,
}

struct FakeTransport {
    replies: Mutex<VecDeque<Reply>>,
    sent: Mutex<Vec<(Method, bool)>>,
}

impl FakeTransport {
    fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
        })
    }
}

fn seconds(count: u64) -> Duration {
    Duration::from_secs(count)
}

fn deadline_in(count: u64) -> Instant {
    Instant::now().checked_add(seconds(count)).unwrap()
}

fn reply(status: u16, body: FakeBody) -> Reply {
    Reply {
        delay: Duration::ZERO,
        outcome: Ok(HttpResponse { status, body }),
    }
}

impl Transport for FakeTransport {
    type Request = Method;
    type Body = FakeBody;
    type Error = FakeError;

    fn method(request: &Method) -> &Method {
        request
    }

    async fn send(
        &self,
        request: Method,
        stream: bool,
    ) -> Result<HttpResponse<FakeBody>, FakeError> {
        self.sent.lock().unwrap().push((request, stream));
        let next = self.replies.lock().unwrap().pop_front().unwrap();
        sleep(next.delay).await;
        next.outcome
    }

    fn status_error(&self, status: u16) -> FakeError {
        FakeError::Status(status)
    }
}
