use std::future::{Future, ready};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_error::envelope::WireField;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::from_str;

use super::super::service::{ListQuery, Operation, ReadCall, ReadProvider};
use super::support::{
    Fixture, REQUEST_TIMEOUT, Which, call_with, listing, moment, record, redactor, supplied,
};
use crate::api::vector_store_http_create_read::error::CreateReadError;
use crate::api::vector_store_http_create_read::service::{
    CreateReadCall, CreateReadDeps, CreateReadTarget, ListCall, RetrieveCall, prepare, send,
};
use crate::api::vector_store_http_write::error::ClientError;
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient, WriteRequest, WriteTransport,
};

#[derive(Clone)]
pub(crate) struct Wire {
    status: u16,
    body: Vec<u8>,
    requests: Arc<Mutex<Vec<WriteRequest>>>,
}

impl Wire {
    fn replying(body: &str) -> Self {
        Self::answering(200, body)
    }

    fn answering(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            requests: Arc::default(),
        }
    }

    fn requests(&self) -> MutexGuard<'_, Vec<WriteRequest>> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl WriteTransport for Wire {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.requests().push(request);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        ready(Ok(UpstreamResponse {
            status: self.status,
            headers,
            body: self.body.clone(),
        }))
    }
}

pub(crate) struct Source {
    wire: Wire,
    calls: Mutex<Vec<Option<bool>>>,
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ssl_verify);
        Ok(self.wire.clone())
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

// B2485 behind the read service: it prepares and sends the same GET the production adapter would
pub(crate) struct Native {
    wire: Wire,
    source: Source,
    redactor: Redactor,
    sink: Sink,
    now: DateTime<FixedOffset>,
    captured: Mutex<Vec<Duration>>,
}

impl Native {
    fn new(wire: Wire) -> Self {
        Self {
            source: Source {
                wire: wire.clone(),
                calls: Mutex::new(Vec::new()),
            },
            wire,
            redactor: redactor(),
            sink: Sink,
            now: moment(),
            captured: Mutex::new(Vec::new()),
        }
    }

    fn captured(&self) -> Vec<Duration> {
        self.captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn requests(&self) -> MutexGuard<'_, Vec<WriteRequest>> {
        self.wire.requests()
    }

    fn source_calls(&self) -> Vec<Option<bool>> {
        self.source
            .calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn pre_post(&self) -> PrePostEnv<'_> {
        PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: self.now,
        }
    }
}

impl ReadProvider for Native {
    type Client = Wire;

    fn read(
        &self,
        call: &ReadCall<'_, Wire>,
    ) -> impl Future<Output = Result<Json, CreateReadError>> + Send {
        self.captured
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.timeout);
        async move {
            let deps = CreateReadDeps {
                config: &call.config,
                source: &self.source,
                supplied: call.client.map(|supplied| SuppliedClient {
                    kind: supplied.kind,
                    transport: supplied.transport,
                }),
            };
            let target = CreateReadTarget {
                params: call.params,
                extra_headers: call.extra_headers,
                timeout: Some(call.timeout),
            };
            let http_call = match call.operation {
                Operation::Retrieve => CreateReadCall::Retrieve(RetrieveCall {
                    target,
                    vector_store_id: call
                        .vector_store_id
                        .expect("a retrieve read carries its vector store id"),
                    extra_body: call.extra_body,
                }),
                Operation::List => CreateReadCall::List(ListCall {
                    target,
                    query: ListQuery {
                        after: call.query.after,
                        before: call.query.before,
                        limit: call.query.limit,
                        order: call.query.order,
                    },
                    extra_body: call.extra_body,
                }),
            };
            let env = self.pre_post();
            let prepared = {
                let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
                prepare(
                    &deps,
                    call.mode,
                    &http_call,
                    &mut LogContext {
                        state: &mut state,
                        env: &env,
                    },
                )
            };
            send(prepared?).await
        }
    }
}

fn authorization(request: &WriteRequest) -> Option<&str> {
    request
        .headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
}

// null takes REQUEST_TIMEOUT's loaded value, zero falls back to it, and 3 keeps 3
#[rstest]
#[case::null(None, REQUEST_TIMEOUT, 6000.0_f64)]
#[case::zero(Some(0.0_f64), REQUEST_TIMEOUT, 6000.0_f64)]
#[case::three(Some(3.0_f64), REQUEST_TIMEOUT, 3.0_f64)]
#[case::configured(None, 4.5_f64, 4.5_f64)]
#[tokio::test]
async fn native_read_timeout(
    #[case] requested: Option<f64>,
    #[case] configured: f64,
    #[case] expected: f64,
) {
    let native = Native::new(Wire::replying(record()));
    let mut fixture = Fixture::new();
    fixture.arguments.timeout = requested;
    call_with(Which::Retrieve, &native, &fixture, None, configured)
        .await
        .expect("the read answers");
    assert_eq!(
        native.captured(),
        vec![Duration::try_from_secs_f64(expected).expect("the fixture is a duration")]
    );
    // the GET never receives the resolved deadline, only the selected client's own
    let requests = native.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].timeout, None);
}

#[tokio::test]
async fn native_retrieve_request() {
    let native = Native::new(Wire::replying(record()));
    let mut fixture = Fixture::new();
    fixture.arguments.extra_query = Some(IndexMap::from([(
        String::from("after"),
        Json::Text(String::from("ignored")),
    )]));
    fixture.arguments.extra_body = Some(IndexMap::from([(
        String::from("name"),
        Json::Text(String::from("ignored")),
    )]));
    fixture.params.api_key = Some(SecretString::from("sk-caller"));
    let value = call_with(Which::Retrieve, &native, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the read answers");
    assert_eq!(value, record());
    let requests = native.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores/vs_a");
    assert_eq!(requests[0].url.query(), None);
    assert!(matches!(requests[0].body, RequestBody::Empty));
    assert_eq!(authorization(&requests[0]), Some("Bearer sk-caller"));
}

#[tokio::test]
async fn native_list_request() {
    // an absent limit and order take the signature's defaults
    let native = Native::new(Wire::replying(listing()));
    let fixture = Fixture::new();
    let value = call_with(Which::List, &native, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the list answers");
    assert_eq!(value, listing());
    let requests = native.requests();
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].url.path(), "/v1/vector_stores");
    assert_eq!(requests[0].url.query(), Some("limit=20&order=desc"));
    assert!(matches!(requests[0].body, RequestBody::Empty));

    // every cursor travels, an empty after and a zero limit included
    let native = Native::new(Wire::replying(listing()));
    let mut fixture = Fixture::new();
    fixture.arguments.after = Some(String::new());
    fixture.arguments.before = Some(String::from("vs_z"));
    fixture.arguments.limit = Some(Some(0_i64));
    fixture.arguments.order = Some(Some(String::from("asc")));
    call_with(Which::List, &native, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the list answers");
    let requests = native.requests();
    assert_eq!(
        requests[0].url.query(),
        Some("after=&before=vs_z&limit=0&order=asc")
    );

    // an explicit null omits both
    let native = Native::new(Wire::replying(listing()));
    let mut fixture = Fixture::new();
    fixture.arguments.limit = Some(None);
    fixture.arguments.order = Some(None);
    call_with(Which::List, &native, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("the list answers");
    assert_eq!(native.requests()[0].url.query(), None);
}

#[tokio::test]
async fn native_selects_the_matching_client() {
    // a sync client in sync mode is used, so the source builds nothing
    let wire = Wire::replying(record());
    let native = Native::new(wire.clone());
    let fixture = Fixture::new();
    call_with(
        Which::Retrieve,
        &native,
        &fixture,
        Some(supplied(ClientKind::Sync, &wire)),
        REQUEST_TIMEOUT,
    )
    .await
    .expect("the read answers");
    assert!(native.source_calls().is_empty());
    assert_eq!(native.requests().len(), 1);

    // an async call skips a sync supplied client and builds the source's
    let wire = Wire::replying(record());
    let native = Native::new(wire.clone());
    let fixture = Fixture::new();
    call_with(
        Which::Aretrieve,
        &native,
        &fixture,
        Some(supplied(ClientKind::Sync, &wire)),
        REQUEST_TIMEOUT,
    )
    .await
    .expect("the read answers");
    assert_eq!(native.source_calls(), vec![None]);

    // the async supplied client is used as given
    let wire = Wire::replying(record());
    let native = Native::new(wire.clone());
    let fixture = Fixture::new();
    call_with(
        Which::Aretrieve,
        &native,
        &fixture,
        Some(supplied(ClientKind::Asynchronous, &wire)),
        REQUEST_TIMEOUT,
    )
    .await
    .expect("the read answers");
    assert!(native.source_calls().is_empty());
}

// a 404 with a body the provider cannot map keeps its status in either mode
#[rstest]
#[case::sync(Which::Retrieve)]
#[case::asynchronous(Which::Aretrieve)]
#[tokio::test]
async fn native_read_failures(#[case] which: Which) {
    let native = Native::new(Wire::answering(404, "[]"));
    let fixture = Fixture::new();
    let error = call_with(which, &native, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect_err("a 404 is an error");
    let raised = error.raised();
    assert_eq!(raised.class, "NotFoundError");
    assert_eq!(raised.status, 404_u16);
    assert_eq!(raised.kind, WireField::Null);
    assert_eq!(raised.param, WireField::Null);
    assert_eq!(raised.code, None);
    assert_eq!(native.requests().len(), 1);
}

// list returns whatever JSON the 404 carries, as LiteLLM's response.json() does
#[tokio::test]
async fn native_list_keeps_a_json_body_whatever_the_status() {
    let native = Native::new(Wire::answering(404, r#"{"error":"gone"}"#));
    let fixture = Fixture::new();
    let value = call_with(Which::List, &native, &fixture, None, REQUEST_TIMEOUT)
        .await
        .expect("a json body is the list's answer");
    let expected: Json = from_str(r#"{"error":"gone"}"#).expect("the fixture is JSON");
    assert_eq!(value, expected);
}
