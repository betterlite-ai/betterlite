use std::collections::{BTreeSet, VecDeque};
use std::future::{Future, ready};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio::sync::Notify;

use crate::catalog::{BundledCatalog, CatalogError, ModelCatalog};
use crate::fetch_policy::client::{
    CatalogClient, CatalogClientError, CatalogRequest, FetchErrorKind,
};
use crate::fetch_policy::response::FetchResponse;
use crate::fetch_policy::retry::{CatalogSleeper, JitterSource};
use crate::fetch_policy::revision::git_blob_id;
use crate::reload::{CatalogService, FetchLimits, NoRuntimeRegistrations, RuntimeRegistrations};
use crate::source::{CatalogClock, CatalogSource, CatalogStore};

pub(crate) const URL: &str = "https://catalog.test/model.json";
pub(crate) const MOMENT: &str = "2026-01-02T03:04:05Z";

pub(crate) type TestService = CatalogService<fn() -> Result<Vec<u8>, CatalogError>>;

pub(crate) fn fixed_moment() -> DateTime<Utc> {
    MOMENT.parse().unwrap()
}

pub(crate) fn fixed_clock() -> CatalogClock {
    Arc::new(fixed_moment)
}

pub(crate) fn blob(body: &str) -> String {
    git_blob_id(body.as_bytes())
}

pub(crate) fn remote_source() -> CatalogSource {
    CatalogSource::Remote {
        url: URL.to_owned(),
    }
}

pub(crate) fn limits(max_attempts: u32) -> FetchLimits {
    FetchLimits {
        timeout: Duration::from_secs(5),
        max_attempts,
    }
}

pub(crate) fn catalog_body(prefix: &str, count: usize) -> String {
    let entries = (0..count)
        .map(|index| {
            format!("\"{prefix}-{index:02}\":{{\"litellm_provider\":\"openai\",\"mode\":\"chat\"}}")
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{entries}}}")
}

pub(crate) fn body_with_entry(prefix: &str, entry: &str) -> String {
    let mut body = catalog_body(prefix, 60);
    body.pop();
    body.push(',');
    body.push_str(entry);
    body.push('}');
    body
}

pub(crate) fn build(
    source: CatalogSource,
    asset: fn() -> Result<Vec<u8>, CatalogError>,
    registrations: Arc<dyn RuntimeRegistrations>,
) -> Arc<TestService> {
    Arc::new(CatalogService {
        source,
        bundled: Mutex::new(BundledCatalog {
            asset,
            cached: None,
        }),
        clock: fixed_clock(),
        registrations,
        current: CatalogStore::default(),
    })
}

pub(crate) fn bundled_service() -> Arc<TestService> {
    let asset: fn() -> Result<Vec<u8>, CatalogError> =
        || Ok(catalog_body("bundled", 60).into_bytes());
    build(remote_source(), asset, Arc::new(NoRuntimeRegistrations))
}

pub(crate) fn local_service() -> Arc<TestService> {
    let asset: fn() -> Result<Vec<u8>, CatalogError> =
        || Ok(catalog_body("bundled", 60).into_bytes());
    build(
        CatalogSource::Local,
        asset,
        Arc::new(NoRuntimeRegistrations),
    )
}

pub(crate) fn wide_bundled_service() -> Arc<TestService> {
    let asset: fn() -> Result<Vec<u8>, CatalogError> =
        || Ok(catalog_body("bundled", 200).into_bytes());
    build(remote_source(), asset, Arc::new(NoRuntimeRegistrations))
}

pub(crate) fn registrations_service(
    registrations: Arc<dyn RuntimeRegistrations>,
) -> Arc<TestService> {
    let asset: fn() -> Result<Vec<u8>, CatalogError> =
        || Ok(catalog_body("bundled", 60).into_bytes());
    build(remote_source(), asset, registrations)
}

#[derive(Clone, Debug)]
pub(crate) enum Reply {
    Response {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Transport {
        detail: String,
    },
    InvalidUrl {
        detail: String,
    },
}

impl Reply {
    pub(crate) fn ok(body: &str) -> Self {
        Self::Response {
            status: 200,
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn ok_with_etag(body: &str, etag: &str) -> Self {
        Self::Response {
            status: 200,
            headers: vec![("ETag".to_owned(), etag.to_owned())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn status(status: u16) -> Self {
        Self::Response {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub(crate) fn transport(detail: &str) -> Self {
        Self::Transport {
            detail: detail.to_owned(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ScriptedError {
    #[error("{detail}")]
    Transport { detail: String },
    #[error("{detail}")]
    InvalidUrl { detail: String },
}

impl CatalogClientError for ScriptedError {
    fn kind(&self) -> FetchErrorKind {
        match self {
            Self::Transport { .. } => FetchErrorKind::Transport,
            Self::InvalidUrl { .. } => FetchErrorKind::InvalidUrl,
        }
    }
}

pub(crate) struct ScriptedClient {
    replies: Mutex<VecDeque<Reply>>,
    requests: AtomicUsize,
    timeouts: Mutex<Vec<Duration>>,
}

impl ScriptedClient {
    pub(crate) fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: AtomicUsize::new(0),
            timeouts: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    pub(crate) fn timeouts(&self) -> Vec<Duration> {
        self.timeouts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn next(&self, request: &CatalogRequest) -> Result<FetchResponse, ScriptedError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.timeouts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.timeout);
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| Reply::Transport {
                detail: "unscripted request".to_owned(),
            });
        match reply {
            Reply::Response {
                status,
                headers,
                body,
            } => Ok(FetchResponse::new(status, headers, body)),
            Reply::Transport { detail } => Err(ScriptedError::Transport { detail }),
            Reply::InvalidUrl { detail } => Err(ScriptedError::InvalidUrl { detail }),
        }
    }
}

impl CatalogClient for ScriptedClient {
    type Error = ScriptedError;

    fn get(&self, request: &CatalogRequest) -> Result<FetchResponse, Self::Error> {
        self.next(request)
    }

    fn get_async(
        &self,
        request: &CatalogRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        ready(self.next(request))
    }
}

#[derive(Default)]
pub(crate) struct RecordingSleeper {
    pub(crate) seconds: Mutex<Vec<Decimal>>,
}

impl RecordingSleeper {
    pub(crate) fn recorded(&self) -> Vec<Decimal> {
        self.seconds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CatalogSleeper for RecordingSleeper {
    fn sleep(&self, seconds: Decimal) -> impl Future<Output = ()> + Send {
        self.seconds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seconds);
        ready(())
    }
}

#[derive(Default)]
pub(crate) struct BlockingSleeper {
    pub(crate) entered: Notify,
    pub(crate) release: Notify,
    pub(crate) seconds: Mutex<Vec<Decimal>>,
}

impl CatalogSleeper for BlockingSleeper {
    fn sleep(&self, seconds: Decimal) -> impl Future<Output = ()> + Send {
        self.seconds
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seconds);
        async move {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
}

pub(crate) struct ScriptedJitter {
    values: VecDeque<Decimal>,
}

impl ScriptedJitter {
    pub(crate) fn new(values: &[Decimal]) -> Self {
        Self {
            values: values.iter().copied().collect(),
        }
    }
}

impl JitterSource for ScriptedJitter {
    fn unit_interval(&mut self) -> Decimal {
        self.values.pop_front().unwrap_or(Decimal::ZERO)
    }
}

#[derive(Default)]
pub(crate) struct RecordingRegistrations {
    pub(crate) applied: AtomicUsize,
    pub(crate) live: Mutex<BTreeSet<String>>,
}

impl RecordingRegistrations {
    pub(crate) fn live(&self) -> BTreeSet<String> {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl RuntimeRegistrations for RecordingRegistrations {
    fn reapply(&self, catalog: &ModelCatalog) {
        self.applied.fetch_add(1, Ordering::SeqCst);
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        live.clear();
        live.extend(catalog.entries.keys().cloned());
        live.insert("runtime-override".to_owned());
    }
}

#[derive(Clone, Default)]
pub(crate) struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl CapturedLog {
    pub(crate) fn text(&self) -> String {
        let bytes = match self.0.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        String::from_utf8(bytes).unwrap()
    }
}

impl io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.lock() {
            Ok(mut guard) => guard.extend_from_slice(buf),
            Err(poisoned) => poisoned.into_inner().extend_from_slice(buf),
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct StopFlag(AtomicBool);

impl StopFlag {
    pub(crate) fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub(crate) fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub(crate) fn stopped(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}
