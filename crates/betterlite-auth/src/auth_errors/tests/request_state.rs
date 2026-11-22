use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, TimeZone as _, Utc};
use rstest::rstest;

use super::support::{Recorder, Reply, identity_with, object};
use crate::auth_errors::conversion::AuthFailure;
use crate::auth_errors::ports::{Level, Logger};
use crate::auth_errors::request_state::{
    BodyReadError, BodySource, DeferredBodyError, PathParamsError, ReceivedClock, RequestState,
    RequestTelemetry, SpanRequest, StateRejected, ensure_parent_span, ensure_received_at,
    read_body_deferred, record_unparsable_body_failure,
};
use crate::resolved_key::auth::ParentOtelSpan;

#[derive(Default)]
struct State {
    received_at: Option<DateTime<Utc>>,
    span: Option<ParentOtelSpan>,
    refuse: bool,
}

impl RequestState for State {
    fn received_at(&self) -> Option<DateTime<Utc>> {
        self.received_at
    }

    fn set_received_at(&mut self, at: DateTime<Utc>) -> Result<(), StateRejected> {
        if self.refuse {
            return Err(StateRejected);
        }
        self.received_at = Some(at);
        Ok(())
    }

    fn has_parent_span(&self) -> bool {
        self.span.is_some()
    }

    fn set_parent_span(&mut self, span: ParentOtelSpan) {
        self.span = Some(span);
    }
}

struct FixedClock(DateTime<Utc>);

impl ReceivedClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

fn moment(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, second).unwrap()
}

#[test]
fn received_at_idempotent_and_assignment_failure() {
    let clock = FixedClock(moment(30));
    let mut stamped = State {
        received_at: Some(moment(1)),
        span: None,
        refuse: false,
    };
    assert_eq!(ensure_received_at(&mut stamped, &clock), moment(1));
    let mut fresh = State::default();
    assert_eq!(ensure_received_at(&mut fresh, &clock), moment(30));
    assert_eq!(fresh.received_at, Some(moment(30)));
    assert_eq!(
        ensure_received_at(&mut fresh, &FixedClock(moment(45))),
        moment(30)
    );
    let mut refusing = State {
        received_at: None,
        span: None,
        refuse: true,
    };
    assert_eq!(ensure_received_at(&mut refusing, &clock), moment(30));
    assert_eq!(refusing.received_at, None);
}

struct Telemetry {
    setter: bool,
    calls: Mutex<Vec<String>>,
}

impl RequestTelemetry for Telemetry {
    fn start_request_span(
        &self,
        start: DateTime<Utc>,
        headers: &BTreeMap<String, String>,
    ) -> ParentOtelSpan {
        let agent = headers.get("user-agent").cloned().unwrap_or_default();
        self.calls
            .lock()
            .unwrap()
            .push(format!("start {} {agent}", start.timestamp()));
        ParentOtelSpan::Json(JsonData::String("created".to_owned()))
    }

    fn has_route_setter(&self) -> bool {
        self.setter
    }

    fn set_route_attributes(
        &self,
        span: &ParentOtelSpan,
        url_path: &str,
        http_route: Option<&str>,
    ) {
        let name = match span {
            ParentOtelSpan::Json(JsonData::String(name)) => name.as_str(),
            ParentOtelSpan::Json(_) | ParentOtelSpan::Opaque => "other",
        };
        let template = http_route.unwrap_or("none");
        self.calls
            .lock()
            .unwrap()
            .push(format!("route {name} {url_path} {template}"));
    }
}

#[rstest]
#[case::legacy_with_setter(true, false, vec!["start 1795262400 curl", "route created /v1/files/f1 /v1/files/{file_id}"])]
#[case::legacy_without_setter(false, false, vec!["start 1795262400 curl"])]
#[case::v2_skips_route(true, true, vec!["start 1795262400 curl"])]
fn span_absent_existing_legacy_and_v2(
    #[case] setter: bool,
    #[case] otel_v2: bool,
    #[case] calls: Vec<&str>,
) {
    let headers = BTreeMap::from([("user-agent".to_owned(), "curl".to_owned())]);
    let request = SpanRequest {
        headers: &headers,
        url_path: "/v1/files/f1",
        route_template: Some("/v1/files/{file_id}"),
        otel_v2,
    };
    let clock = FixedClock(moment(0));
    let telemetry = Telemetry {
        setter,
        calls: Mutex::new(Vec::new()),
    };
    let mut absent = State::default();
    ensure_parent_span(&mut absent, &clock, None, &request);
    assert_eq!(
        (absent.received_at, absent.span.is_none()),
        (Some(moment(0)), true)
    );
    let mut existing = State {
        received_at: None,
        span: Some(ParentOtelSpan::Json(JsonData::String("kept".to_owned()))),
        refuse: false,
    };
    ensure_parent_span(&mut existing, &clock, Some(&telemetry), &request);
    assert!(
        matches!(&existing.span, Some(ParentOtelSpan::Json(JsonData::String(name))) if name == "kept")
    );
    assert!(telemetry.calls.lock().unwrap().is_empty());
    let mut fresh = State::default();
    ensure_parent_span(&mut fresh, &clock, Some(&telemetry), &request);
    assert!(
        matches!(&fresh.span, Some(ParentOtelSpan::Json(JsonData::String(name))) if name == "created")
    );
    assert_eq!(*telemetry.calls.lock().unwrap(), calls);
}

struct Source {
    opaque: bool,
    read: Option<Result<JsonObject, BodyReadError>>,
    path_params: Result<(), PathParamsError>,
    log: Vec<String>,
}

impl Source {
    fn reading(read: Result<JsonObject, BodyReadError>) -> Self {
        Self {
            opaque: false,
            read: Some(read),
            path_params: Ok(()),
            log: Vec::new(),
        }
    }
}

#[async_trait]
impl BodySource for Source {
    fn is_opaque_audio(&self) -> bool {
        self.opaque
    }

    fn cache_parsed_body(&mut self, body: JsonObject) {
        self.log.push(format!("cache {}", body.len()));
    }

    async fn read_body(&mut self) -> Result<JsonObject, BodyReadError> {
        self.log.push("read".to_owned());
        self.read.take().unwrap_or(Err(BodyReadError::Unreadable))
    }

    fn populate_path_params(&self, body: JsonObject) -> Result<JsonObject, PathParamsError> {
        match self.path_params {
            Ok(()) => {
                let mut populated = body;
                populated.insert("file_id".to_owned(), Json::Text("f1".to_owned()));
                Ok(populated)
            }
            Err(PathParamsError) => Err(PathParamsError),
        }
    }
}

fn parse_failure() -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "Invalid JSON payload".to_owned(),
        error_type: "invalid_request_error".to_owned(),
        param: Some("request_body".to_owned()),
        code: ExceptionCode::Status(400),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

#[tokio::test]
async fn opaque_audio_empty_cache_without_read() {
    let mut source = Source::reading(Ok(object(&[("model", Json::Null)])));
    source.opaque = true;
    let deferred = read_body_deferred(&mut source).await.unwrap();
    assert!(deferred.body.is_empty());
    assert_eq!(deferred.parse_failure, None);
    assert_eq!(source.log, vec!["cache 0".to_owned()]);
}

#[tokio::test]
async fn proxy_parse_error_deferred_other_errors_escape() {
    let mut failing = Source::reading(Err(BodyReadError::Proxy(Box::new(parse_failure()))));
    let deferred = read_body_deferred(&mut failing).await.unwrap();
    assert!(deferred.body.is_empty());
    assert_eq!(deferred.parse_failure, Some(parse_failure()));
    let mut broken = Source::reading(Err(BodyReadError::Unreadable));
    let escaped = read_body_deferred(&mut broken).await;
    assert!(matches!(escaped, Err(DeferredBodyError::Unreadable)));
    assert_eq!(broken.log, vec!["read".to_owned()]);
}

#[tokio::test]
async fn path_population_error_not_deferred() {
    let mut populated = Source::reading(Ok(object(&[("model", Json::Null)])));
    let deferred = read_body_deferred(&mut populated).await.unwrap();
    assert_eq!(
        deferred.body.get("file_id"),
        Some(&Json::Text("f1".to_owned()))
    );
    assert_eq!(deferred.parse_failure, None);
    let mut failing = Source::reading(Ok(JsonObject::new()));
    failing.path_params = Err(PathParamsError);
    let escaped = read_body_deferred(&mut failing).await;
    assert!(matches!(
        escaped,
        Err(DeferredBodyError::PathParams(PathParamsError))
    ));
}

#[rstest]
#[case::hook_raises(Reply::Raise, 1)]
#[case::hook_answers(Reply::Original, 0)]
#[tokio::test]
async fn malformed_body_hook_failure_preserves_original_400(
    #[case] reply: Reply,
    #[case] log_count: usize,
) {
    let ports = Recorder::replying(reply);
    let identity = identity_with(&[("user_id", "known-user")]);
    record_unparsable_body_failure(&ports, &identity, &parse_failure(), "/v1/chat/completions")
        .await;
    let calls = ports.hook_calls();
    let Some((request_data, original, error_type, route)) = calls.first() else {
        panic!("the failure hook runs once");
    };
    assert!(request_data.is_empty());
    assert_eq!(*error_type, ProxyErrorTypes::BadRequestError);
    assert_eq!(route, "/v1/chat/completions");
    assert_eq!(
        original,
        &AuthFailure::Proxy {
            exception: parse_failure(),
            invalid_virtual_key: false
        }
    );
    let logs = ports.logs();
    assert_eq!(logs.len(), log_count);
    if let Some(entry) = logs.first() {
        assert_eq!(
            (entry.logger, entry.level, entry.traceback),
            (Logger::Proxy, Level::Error, true)
        );
        assert!(
            entry
                .message
                .starts_with("Failed to log the request rejected for an unparsable body")
        );
    }
}
