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
use tracing::Level;
use tracing::subscriber::with_default;

use super::super::recorder::{BillingRecorder, BillingRecorderError};
use super::super::sink::{GatewayRequestSink, GatewayRequestSinkError};
use super::super::{BillableRequestMetrics, RecorderFactory, SinkFactory, record_billable_request};

type SinkCall = (BillableCategory, String, StatusCode);
type RecorderCall = (BillableCategory, String, StatusCode, Option<String>);

fn record_or_fail<T>(
    calls: &Mutex<Vec<T>>,
    call: T,
    fails: bool,
    failure: &'static str,
) -> Result<(), io::Error> {
    calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(call);
    if fails {
        return Err(io::Error::other(failure));
    }
    Ok(())
}

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

macro_rules! define_mock {
    ($name:ident, $trait:ident, $call:ty, $error:ty, $ctor:expr, $failure:literal, ($($arg:ident: $ty:ty),+), $tuple:expr) => {
        pub(crate) struct $name {
            calls: Mutex<Vec<$call>>,
            fails: bool,
        }

        impl $name {
            pub(crate) fn new(fails: bool) -> Arc<Self> {
                Arc::new(Self {
                    calls: Mutex::new(Vec::new()),
                    fails,
                })
            }

            pub(crate) fn calls(&self) -> Vec<$call> {
                self.calls
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            }
        }

        impl $trait for $name {
            fn record(&self, $($arg: $ty),+) -> Result<(), $error> {
                record_or_fail(&self.calls, $tuple, self.fails, $failure).map_err($ctor)
            }
        }
    };
}

define_mock!(
    MockSink,
    GatewayRequestSink,
    SinkCall,
    GatewayRequestSinkError,
    GatewayRequestSinkError::Failed,
    "mock sink failure",
    (category: BillableCategory, route: &str, status_code: StatusCode),
    (category, route.to_owned(), status_code)
);

define_mock!(
    MockRecorder,
    BillingRecorder,
    RecorderCall,
    BillingRecorderError,
    BillingRecorderError::Failed,
    "mock recorder failure",
    (
        category: BillableCategory,
        route: &str,
        status_code: StatusCode,
        model_id: Option<&str>
    ),
    (category, route.to_owned(), status_code, model_id.map(str::to_owned))
);

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
