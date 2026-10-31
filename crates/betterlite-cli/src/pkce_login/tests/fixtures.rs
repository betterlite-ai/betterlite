use secrecy::ExposeSecret as _;
use std::collections::VecDeque;
use std::io::{Error as IoError, ErrorKind};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::super::{
    Clock, Entropy, Http, HttpBody, HttpResponse, WallClock,
    loopback::{LoopbackConnection, LoopbackListener},
    models::PkceFailure,
};

pub(super) struct FixedEntropy;

pub(super) struct FixedClock(pub(super) Instant);

impl Clock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }
}

pub(super) struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub(super) struct FixedWallClock(pub(super) i64);

impl WallClock for FixedWallClock {
    fn now_seconds(&self) -> i64 {
        self.0
    }
}

// Shares a counter with a transport double that advances it, to catch a clock read before the call.
pub(super) struct SharedWallClock(pub(super) Arc<AtomicI64>);

impl WallClock for SharedWallClock {
    fn now_seconds(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl Entropy for FixedEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<(), PkceFailure> {
        for byte in bytes {
            *byte = 1;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct ScriptedHttp {
    response: HttpResponse,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    clock_advance: Option<(Arc<AtomicI64>, i64)>,
}

#[derive(Clone)]
pub(super) struct RecordedRequest {
    pub(super) url: String,
    pub(super) body: HttpBody,
    pub(super) timeout: u64,
    pub(super) follow_redirects: bool,
}

#[derive(serde::Deserialize)]
pub(super) struct RegistrationPayload {
    pub(super) client_name: String,
    pub(super) redirect_uris: Vec<String>,
    pub(super) grant_types: Vec<String>,
    pub(super) response_types: Vec<String>,
    pub(super) token_endpoint_auth_method: String,
}

impl ScriptedHttp {
    pub(super) fn new(response: HttpResponse) -> Self {
        Self {
            response,
            requests: Arc::new(Mutex::new(Vec::new())),
            clock_advance: None,
        }
    }

    // Bumps a shared wall-clock counter as a side effect of answering, so a test can tell whether a caller sampled the clock before or after this transport's call.
    pub(super) fn with_clock_advance(mut self, clock: Arc<AtomicI64>, advance_by: i64) -> Self {
        self.clock_advance = Some((clock, advance_by));
        self
    }

    pub(super) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().expect("requests").clone()
    }

    fn record(
        &self,
        url: &str,
        body: HttpBody,
        timeout_seconds: u64,
        follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        if let Some((clock, advance_by)) = &self.clock_advance {
            clock.fetch_add(*advance_by, Ordering::SeqCst);
        }
        self.requests
            .lock()
            .map_err(|_poisoned| PkceFailure::Protocol("request recorder was poisoned".to_owned()))?
            .push(RecordedRequest {
                url: url.to_owned(),
                body,
                timeout: timeout_seconds,
                follow_redirects,
            });
        Ok(self.response.clone())
    }
}

impl Http for ScriptedHttp {
    fn get(&self, url: &str, timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        self.record(url, HttpBody::Json(Vec::new()), timeout_seconds, false)
    }

    fn post(
        &self,
        url: &str,
        body: HttpBody,
        timeout_seconds: u64,
        follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        self.record(url, body, timeout_seconds, follow_redirects)
    }
}

// Reads the values out of a `HttpBody::Form`; its values are `SecretString` so a derived `PartialEq` cannot compare them.
pub(super) fn form_fields(body: &HttpBody) -> Vec<(String, String)> {
    let HttpBody::Form(fields) = body else {
        panic!("expected a form body");
    };
    fields
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

pub(super) struct FailingListener {
    pub(super) address: SocketAddr,
    pub(super) attempts: Mutex<VecDeque<Result<Option<RecordedConnection>, ErrorKind>>>,
}

impl LoopbackListener for FailingListener {
    type Connection = RecordedConnection;

    fn local_addr(&self) -> Result<SocketAddr, IoError> {
        Ok(self.address)
    }

    fn accept(&self) -> Result<Option<Self::Connection>, IoError> {
        let mut attempts = self
            .attempts
            .lock()
            .map_err(|_poisoned| IoError::other("attempts queue was poisoned"))?;
        match attempts.pop_front() {
            Some(Ok(connection)) => Ok(connection),
            Some(Err(kind)) => Err(IoError::from(kind)),
            None => Ok(None),
        }
    }
}

pub(super) struct RecordedConnection {
    pub(super) reads: VecDeque<Result<Vec<u8>, ErrorKind>>,
    pub(super) write_result: Result<(), ErrorKind>,
}

impl LoopbackConnection for RecordedConnection {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, IoError> {
        match self.reads.pop_front() {
            Some(Ok(bytes)) => {
                let size = bytes.len().min(buffer.len());
                let (Some(destination), Some(source)) = (buffer.get_mut(..size), bytes.get(..size))
                else {
                    return Err(IoError::other("short read buffer"));
                };
                destination.copy_from_slice(source);
                Ok(size)
            }
            Some(Err(kind)) => Err(IoError::from(kind)),
            None => Ok(0),
        }
    }

    fn write_all(&mut self, _bytes: &[u8]) -> Result<(), IoError> {
        match self.write_result {
            Ok(()) => Ok(()),
            Err(kind) => Err(IoError::from(kind)),
        }
    }
}

pub(super) fn redirect_response(status: u16) -> ScriptedHttp {
    ScriptedHttp::new(HttpResponse {
        status,
        headers: Vec::new(),
        body: Vec::new(),
    })
}
