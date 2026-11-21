#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::auth_request_policy::Settings;
use betterlite_config::sections::request_body_safety::ClientsideParamGrant;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use tracing::subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::{Layer, Registry};

use super::bytes::{SizeError, check_request_size, check_response_size};
use super::invalid_key::{
    ErrorClass, ErrorFacts, MarkOutcome, Marker, is_invalid_virtual_key_error,
    mark_invalid_virtual_key_error,
};
use super::ip::{RequestProvenance, check_valid_ip, request_ip_address};
use super::policy::{
    PolicyError, PreDbRequest, RouteDecision, check_allowed_route, pre_db_read_auth_checks,
};
use crate::request_body_safety::{
    FormMetadataExtractor, ModelGrantLookup, ModelGrants, RequestBodySafety,
};

const ROUTE: &str = "/v1/chat/completions";
const BANNED_BODY: &[u8] = br#"{"model":"gpt-4o","api_base":"https://evil.example"}"#;

#[derive(Debug, Default)]
struct Grants(BTreeMap<String, Option<Vec<ClientsideParamGrant>>>);

impl ModelGrantLookup for Grants {
    fn grants(&self, model_group: &str) -> ModelGrants<'_> {
        match self.0.get(model_group) {
            None => ModelGrants::Absent,
            Some(None) => ModelGrants::Unset,
            Some(Some(items)) => ModelGrants::Granted(items),
        }
    }
}

#[derive(Debug)]
struct Metadata;

impl FormMetadataExtractor for Metadata {
    fn top_level_names(&self, form_data: &JsonObject, prefix: &str) -> Vec<String> {
        form_data
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect()
    }
}

fn provenance<'request>(
    forwarded_for: Option<&'request str>,
    client_host: Option<&'request str>,
) -> RequestProvenance<'request> {
    RequestProvenance {
        forwarded_for,
        client_host,
    }
}

fn ip_settings(allowed_ips: &[&str], use_x_forwarded_for: bool) -> Settings {
    Settings {
        allowed_ips: Some(allowed_ips.iter().map(|ip| String::from(*ip)).collect()),
        use_x_forwarded_for,
        max_request_size_mb: None,
        max_response_size_mb: None,
        allowed_routes: None,
    }
}

fn route_settings(allowed_routes: &[&str]) -> Settings {
    Settings {
        allowed_ips: None,
        use_x_forwarded_for: false,
        max_request_size_mb: None,
        max_response_size_mb: None,
        allowed_routes: Some(
            allowed_routes
                .iter()
                .map(|route| String::from(*route))
                .collect(),
        ),
    }
}

fn policy_request<'scan>(
    route: &'scan str,
    provenance: RequestProvenance<'scan>,
) -> PreDbRequest<'scan> {
    PreDbRequest {
        content_length: None,
        body: &[],
        route,
        provenance,
        body_safety: None,
    }
}

fn safety_request<'scan>(
    body_bytes: &'scan [u8],
    parsed: &'scan JsonObject,
    grants: &'scan Grants,
    metadata: &'scan Metadata,
    provenance: RequestProvenance<'scan>,
) -> PreDbRequest<'scan> {
    let model = match parsed.get("model") {
        Some(Json::Text(model)) => model.as_str(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => "",
    };
    let mut request = policy_request(ROUTE, provenance);
    request.body = body_bytes;
    request.body_safety = Some(RequestBodySafety {
        body: parsed,
        model,
        allow_client_side_credentials: false,
        allowed_hosts: &[],
        grants,
        metadata,
        max_fallbacks: 5,
    });
    request
}

fn denial(settings: &Settings, provenance: RequestProvenance<'_>) -> PolicyError {
    pre_db_read_auth_checks(policy_request(ROUTE, provenance), settings).unwrap_err()
}

fn proxy_exception(code: &str) -> ProxyException {
    ProxyException::new(ProxyExceptionInit {
        message: "Invalid proxy server token passed".to_owned(),
        error_type: "auth_error".to_owned(),
        param: Some("None".to_owned()),
        code: ExceptionCode::Text(code.to_owned()),
        headers: BTreeMap::new(),
        openai_code: None,
        provider_specific_fields: BTreeMap::new(),
    })
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<String>>>);

struct RecordedLog(String);

impl tracing::field::Visit for RecordedLog {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: tracing::Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, Inner>) {
        let mut recorded = RecordedLog(String::new());
        event.record(&mut recorded);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{} {}", event.metadata().level(), recorded.0));
    }
}

fn route_events(allowed_routes: &[&str]) -> Vec<String> {
    let logs = Logs::default();
    let subscriber = Registry::default().with(logs.clone());
    let guard = subscriber::set_default(subscriber);
    let error = denial(
        &route_settings(allowed_routes),
        provenance(None, Some("10.0.0.9")),
    );
    assert!(matches!(error, PolicyError::RouteDenied { .. }));
    drop(guard);
    logs.0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[test]
fn size_before_body_safety() {
    let settings = Settings {
        allowed_ips: None,
        use_x_forwarded_for: false,
        max_request_size_mb: Some(0.0_f64),
        max_response_size_mb: None,
        allowed_routes: None,
    };
    let grants = Grants::default();
    let metadata = Metadata;
    let parsed: JsonObject = serde_json::from_slice(BANNED_BODY).unwrap();
    let request = safety_request(
        BANNED_BODY,
        &parsed,
        &grants,
        &metadata,
        provenance(None, Some("10.0.0.9")),
    );
    let error = pre_db_read_auth_checks(request, &settings).unwrap_err();
    assert!(matches!(
        error,
        PolicyError::Size(SizeError::RequestTooLarge { .. })
    ));
}

#[test]
fn body_before_ip() {
    let settings = ip_settings(&["10.0.0.1"], false);
    let grants = Grants::default();
    let metadata = Metadata;
    let parsed: JsonObject = serde_json::from_slice(BANNED_BODY).unwrap();
    let request = safety_request(
        BANNED_BODY,
        &parsed,
        &grants,
        &metadata,
        provenance(None, Some("203.0.113.9")),
    );
    let error = pre_db_read_auth_checks(request, &settings).unwrap_err();
    assert!(matches!(error, PolicyError::BodySafety(_)));
}

#[test]
fn ip_before_routes() {
    let settings = Settings {
        allowed_ips: Some(vec!["10.0.0.1".to_owned()]),
        use_x_forwarded_for: false,
        max_request_size_mb: None,
        max_response_size_mb: None,
        allowed_routes: Some(vec!["/v1/embeddings".to_owned()]),
    };
    let error = denial(&settings, provenance(None, Some("203.0.113.9")));
    assert!(matches!(error, PolicyError::IpDenied { client_ip } if client_ip == "203.0.113.9"));
}

#[test]
fn allowed_ips_none_skips_ip() {
    let request = policy_request(ROUTE, provenance(None, None));
    pre_db_read_auth_checks(request, &Settings::default()).unwrap();
    let verdict = check_valid_ip(None, &provenance(None, None), true);
    assert_eq!(verdict.passed_in_ip, None);
    assert!(verdict.allowed);
}

#[test]
fn empty_ip_list_denies() {
    let error = denial(&ip_settings(&[], false), provenance(None, None));
    assert!(matches!(error, PolicyError::IpDenied { client_ip } if client_ip.is_empty()));
}

#[test]
fn forwarded_header_entire_value() {
    let combined = "10.0.0.1, 203.0.113.7";
    let request = policy_request(ROUTE, provenance(Some(combined), Some("10.0.0.9")));
    pre_db_read_auth_checks(request, &ip_settings(&[combined], true)).unwrap();
    // the header is one string, so the first hop alone does not match
    let error = denial(
        &ip_settings(&["10.0.0.1"], true),
        provenance(Some(combined), Some("10.0.0.9")),
    );
    assert!(matches!(error, PolicyError::IpDenied { .. }));
}

#[test]
fn forwarded_empty_value() {
    let request = policy_request(ROUTE, provenance(Some(""), Some("10.0.0.9")));
    pre_db_read_auth_checks(request, &ip_settings(&[""], true)).unwrap();
    let error = denial(
        &ip_settings(&["10.0.0.9"], true),
        provenance(Some(""), Some("10.0.0.9")),
    );
    assert!(matches!(error, PolicyError::IpDenied { client_ip } if client_ip.is_empty()));
}

#[test]
fn forwarded_toggle_exact_true() {
    let request = policy_request(ROUTE, provenance(Some("203.0.113.7"), Some("10.0.0.9")));
    pre_db_read_auth_checks(request, &ip_settings(&["10.0.0.9"], false)).unwrap();
    let error = denial(
        &ip_settings(&["10.0.0.9"], true),
        provenance(Some("203.0.113.7"), Some("10.0.0.9")),
    );
    assert!(matches!(error, PolicyError::IpDenied { client_ip } if client_ip == "203.0.113.7"));
}

#[test]
fn client_host_fallback() {
    let request = policy_request(ROUTE, provenance(None, Some("10.0.0.9")));
    pre_db_read_auth_checks(request, &ip_settings(&["10.0.0.9"], true)).unwrap();
}

#[test]
fn no_client_empty_ip() {
    let request = policy_request(ROUTE, provenance(None, None));
    pre_db_read_auth_checks(request, &ip_settings(&[""], true)).unwrap();
    assert_eq!(request_ip_address(&provenance(None, None), true), "");
}

#[test]
fn routes_absent_vs_empty() {
    pre_db_read_auth_checks(
        policy_request(ROUTE, provenance(None, None)),
        &Settings::default(),
    )
    .unwrap();
    let error = denial(&route_settings(&[]), provenance(None, None));
    assert!(matches!(error, PolicyError::RouteDenied { route } if route == ROUTE));
}

#[test]
fn exact_route_no_wildcards() {
    let error = denial(
        &route_settings(&["/v1/*", "/v1/chat"]),
        provenance(None, None),
    );
    assert!(matches!(error, PolicyError::RouteDenied { .. }));
    pre_db_read_auth_checks(
        policy_request(ROUTE, provenance(None, None)),
        &route_settings(&[ROUTE]),
    )
    .unwrap();
    let wildcard = ["/v1/*".to_owned()];
    assert_eq!(
        check_allowed_route(Some(&wildcard), ROUTE),
        RouteDecision::Denied
    );
}

#[test]
fn route_miss_logs() {
    let events = route_events(&["/v1/embeddings"]);
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].starts_with("ERROR"), "{events:?}");
    assert!(events[0].contains("allowed_routes"), "{events:?}");
}

#[test]
fn no_route_license_diagnostic() {
    let events = route_events(&["/v1/embeddings"]);
    let text = events.join("\n").to_lowercase();
    assert!(!text.contains("license"), "{text}");
    assert!(!text.contains("enterprise"), "{text}");
    let exception = PolicyError::RouteDenied {
        route: ROUTE.to_owned(),
    }
    .into_proxy_exception();
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "403");
}

#[test]
fn no_size_limit() {
    check_request_size(Some("999999999"), &[0_u8; 1024], None).unwrap();
    check_response_size(&[0_u8; 1024], None).unwrap();
}

#[test]
fn mb_is_1048576() {
    // 1_000_001 bytes is over a decimal megabyte and under a mebibyte, so only the 1024 divisor allows it
    check_request_size(Some("1000001"), &[], Some(1.0_f64)).unwrap();
    check_request_size(Some(""), &vec![0_u8; 1_000_001], Some(1.0_f64)).unwrap();
    check_response_size(&vec![0_u8; 1_000_001], Some(1.0_f64)).unwrap();
    check_request_size(Some("1048576"), &[], Some(1.0_f64)).unwrap();
    check_response_size(&vec![0_u8; 524_288], Some(0.5_f64)).unwrap();
    let error = check_response_size(&vec![0_u8; 524_289], Some(0.5_f64)).unwrap_err();
    assert!(matches!(error, SizeError::ResponseTooLarge { .. }));
    let error = check_request_size(Some("1048577"), &[], Some(1.0_f64)).unwrap_err();
    assert!(matches!(error, SizeError::RequestTooLarge { .. }));
}

#[test]
fn header_prevents_body_read() {
    // the header wins even though the body alone would exceed the limit
    let body = vec![0_u8; 2_097_152];
    check_request_size(Some("100"), &body, Some(1.0_f64)).unwrap();
    let error = check_request_size(Some("1048577"), &[], Some(1.0_f64)).unwrap_err();
    assert!(matches!(error, SizeError::RequestTooLarge { .. }));
}

#[test]
fn empty_header_reads_body() {
    let body = vec![0_u8; 1_048_577];
    let error = check_request_size(Some(""), &body, Some(1.0_f64)).unwrap_err();
    assert!(matches!(error, SizeError::RequestTooLarge { .. }));
    check_request_size(Some(""), &[0_u8; 1024], Some(1.0_f64)).unwrap();
}

#[test]
fn equal_size_allowed() {
    check_request_size(Some("1048576"), &[], Some(1.0_f64)).unwrap();
    let error = check_request_size(Some("1048577"), &[], Some(1.0_f64)).unwrap_err();
    assert!(matches!(error, SizeError::RequestTooLarge { .. }));
}

#[test]
fn positive_size_zero_limit() {
    check_request_size(Some("0"), &[], Some(0.0_f64)).unwrap();
    let error = check_request_size(Some("1"), &[], Some(0.0_f64)).unwrap_err();
    assert!(matches!(error, SizeError::RequestTooLarge { .. }));
}

#[test]
fn negative_limit_not_clamped() {
    check_request_size(Some("0"), &[], Some(-1.0_f64)).unwrap();
    let error = check_request_size(Some("1"), &[], Some(-1.0_f64)).unwrap_err();
    assert!(matches!(error, SizeError::RequestTooLarge { .. }));
}

#[test]
fn source_invalid_length_error() {
    let error = check_request_size(Some("not-a-number"), &[], Some(1.0_f64)).unwrap_err();
    let SizeError::InvalidContentLength { value, source } = error else {
        panic!("an unusable header propagates the conversion error");
    };
    assert_eq!(value, "not-a-number");
    assert_eq!(source.to_string(), "invalid digit found in string");
    let exception = SizeError::InvalidContentLength { value, source }.into_proxy_exception();
    assert_eq!(exception.error_type, "internal_server_error");
    assert_eq!(exception.code, "500");
}

#[test]
fn only_supported_error_classes() {
    let facts = ErrorFacts {
        class: ErrorClass::Other,
        code: Some("401"),
        status_code: Some(401),
        marker: Marker::ExactTrue,
    };
    assert!(!is_invalid_virtual_key_error(&facts));
}

#[test]
fn code_precedes_status() {
    let facts = ErrorFacts {
        class: ErrorClass::Http,
        code: Some("403"),
        status_code: Some(401),
        marker: Marker::ExactTrue,
    };
    assert!(!is_invalid_virtual_key_error(&facts));
}

#[test]
fn string_401_matches() {
    let by_code = ErrorFacts {
        class: ErrorClass::Http,
        code: Some("401"),
        status_code: Some(400),
        marker: Marker::ExactTrue,
    };
    assert!(is_invalid_virtual_key_error(&by_code));
    let by_status = ErrorFacts {
        class: ErrorClass::Http,
        code: None,
        status_code: Some(401),
        marker: Marker::ExactTrue,
    };
    assert!(is_invalid_virtual_key_error(&by_status));
    let missing = ErrorFacts {
        class: ErrorClass::Http,
        code: None,
        status_code: None,
        marker: Marker::ExactTrue,
    };
    assert!(!is_invalid_virtual_key_error(&missing));
}

#[test]
fn marker_exact_true() {
    let exception = proxy_exception("401");
    assert!(is_invalid_virtual_key_error(&ErrorFacts::marked(
        &exception
    )));
    assert!(!is_invalid_virtual_key_error(&ErrorFacts {
        marker: Marker::NotTrue,
        ..ErrorFacts::proxy(&exception)
    }));
    assert!(!is_invalid_virtual_key_error(&ErrorFacts::proxy(
        &exception
    )));
}

#[test]
fn message_cannot_mark_error() {
    // a 401 whose message looks like a malformed-key rejection stays unclassified without the marker
    let mut exception = proxy_exception("401");
    exception.message = "Invalid proxy server token passed. Received API Key = sk-...".to_owned();
    assert!(!is_invalid_virtual_key_error(&ErrorFacts::proxy(
        &exception
    )));
}

#[test]
fn unmarked_same_error() {
    let exception = proxy_exception("401");
    assert_eq!(
        mark_invalid_virtual_key_error(&exception, false),
        MarkOutcome::Same
    );
}

#[test]
fn non401_same_error() {
    let exception = proxy_exception("403");
    assert_eq!(
        mark_invalid_virtual_key_error(&exception, true),
        MarkOutcome::Same
    );
}

#[test]
fn marked_copy_headers() {
    let mut exception = proxy_exception("401");
    exception
        .headers
        .insert("x-trace".to_owned(), "abc".to_owned());
    let MarkOutcome::Marked(marked) = mark_invalid_virtual_key_error(&exception, true) else {
        panic!("a 401 is marked");
    };
    assert_eq!(marked.headers, exception.headers);
    assert_eq!(exception.headers.len(), 1);
    assert_eq!(marked.message, exception.message);
    assert_eq!(marked.error_type, exception.error_type);
    assert_eq!(marked.param, exception.param);
    assert_eq!(marked.code, exception.code);
}

#[test]
fn marked_openai_code_string() {
    let mut exception = proxy_exception("401");
    exception.openai_code = Some("invalid_key".to_owned());
    let outcome = mark_invalid_virtual_key_error(&exception, true);
    let MarkOutcome::Marked(marked) = &outcome else {
        panic!("a 401 is marked");
    };
    assert_eq!(marked.openai_code.as_deref(), Some("invalid_key"));
    assert_eq!(outcome.marker(), Marker::ExactTrue);
    assert!(is_invalid_virtual_key_error(&ErrorFacts::marked(marked)));
    assert_eq!(exception.openai_code.as_deref(), Some("invalid_key"));
}

#[test]
fn size_checks_without_license() {
    // the MIT build enforces a configured limit with no license flag in reach
    let error = check_request_size(Some("1048577"), &[], Some(1.0_f64)).unwrap_err();
    let exception = error.into_proxy_exception();
    assert_eq!(exception.error_type, "bad_request_error");
    assert_eq!(exception.param.as_deref(), Some("content-length"));
    assert_eq!(exception.code, "400");
    check_request_size(Some("1048576"), &[], Some(1.0_f64)).unwrap();
}

#[test]
fn response_wire_byte_limit() {
    check_response_size(&vec![0_u8; 1_048_576], Some(1.0_f64)).unwrap();
    let error = check_response_size(&vec![0_u8; 1_048_577], Some(1.0_f64)).unwrap_err();
    let exception = error.into_proxy_exception();
    assert_eq!(exception.error_type, "bad_request_error");
    assert_eq!(exception.param.as_deref(), Some("content-length"));
    assert_eq!(exception.code, "400");
}
