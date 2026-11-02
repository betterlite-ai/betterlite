use std::fs;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode};
use axum::middleware;
use axum::response::Response;
use betterlite_billing::billable_category::BillableCategory;
use tempfile::NamedTempFile;
use tower::BoxError;
use tracing::Level;
use tracing::subscriber::with_default;

use super::super::recorder::BillingRecorder;
use super::super::sink::GatewayRequestSink;
use super::super::{BillableRequestMetrics, RecorderFactory, SinkFactory, record_billable_request};

type SinkCall = (BillableCategory, String, StatusCode);
type RecorderCall = (BillableCategory, String, StatusCode, Option<String>);

pub(crate) fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

pub(crate) struct MockSink {
    calls: Mutex<Vec<SinkCall>>,
    fails: bool,
}

impl MockSink {
    pub(crate) fn new(fails: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            fails,
        })
    }

    pub(crate) fn calls(&self) -> Vec<SinkCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl GatewayRequestSink for MockSink {
    fn record(
        &self,
        category: BillableCategory,
        route: &str,
        status_code: StatusCode,
    ) -> Result<(), BoxError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((category, route.to_owned(), status_code));
        if self.fails {
            return Err(Box::new(io::Error::other("mock sink failure")));
        }
        Ok(())
    }
}

pub(crate) struct MockRecorder {
    calls: Mutex<Vec<RecorderCall>>,
    fails: bool,
}

impl MockRecorder {
    pub(crate) fn new(fails: bool) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            fails,
        })
    }

    pub(crate) fn calls(&self) -> Vec<RecorderCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BillingRecorder for MockRecorder {
    fn record(
        &self,
        category: BillableCategory,
        route: &str,
        status_code: StatusCode,
        model_id: Option<&str>,
    ) -> Result<(), BoxError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                category,
                route.to_owned(),
                status_code,
                model_id.map(str::to_owned),
            ));
        if self.fails {
            return Err(Box::new(io::Error::other("mock recorder failure")));
        }
        Ok(())
    }
}

pub(crate) fn sink_factory(sink: Arc<MockSink>) -> SinkFactory {
    Box::new(move || {
        let concrete: Arc<MockSink> = Arc::clone(&sink);
        let resolved: Arc<dyn GatewayRequestSink> = concrete;
        Some(resolved)
    })
}

pub(crate) fn recorder_factory(recorder: Arc<MockRecorder>) -> RecorderFactory {
    Box::new(move || {
        let concrete: Arc<MockRecorder> = Arc::clone(&recorder);
        let resolved: Arc<dyn BillingRecorder> = concrete;
        Some(resolved)
    })
}

pub(crate) fn app(
    state: BillableRequestMetrics,
    status: StatusCode,
    model_id: Option<&'static str>,
) -> Router {
    Router::new()
        .fallback(move || async move { backend_response(status, model_id) })
        .layer(middleware::from_fn_with_state(
            Arc::new(state),
            record_billable_request,
        ))
}

fn backend_response(status: StatusCode, model_id: Option<&'static str>) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    if let Some(id) = model_id {
        response
            .headers_mut()
            .insert("x-litellm-model-id", HeaderValue::from_static(id));
    }
    response
}
