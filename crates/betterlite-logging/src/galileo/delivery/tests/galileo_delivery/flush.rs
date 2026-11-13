use std::sync::Arc;

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::super::super::buffer::append;
use super::super::super::flush::{FlushOutcome, deliver, ingest_request};
use super::super::super::transport::TraceIds as _;
use super::{
    API_KEY_ENV, CountingIds, ScriptedTransport, TOKEN_ENV, cached_header_names, capture_logs,
    delivery, labels, record, response, unreachable_host,
};

const LOGIN_REPLY: &str = r#"{"access_token": "tok-123"}"#;

fn traces_of(payload: &super::Record) -> Vec<Json> {
    match payload.get("traces") {
        Some(Json::List(traces)) => traces.clone(),
        other => panic!("payload has no traces list: {other:?}"),
    }
}

#[test]
fn the_v2_api_ingests_under_the_project_id() {
    let delivery = delivery(API_KEY_ENV);
    let records = [record("a"), record("b")];
    let (url, payload) = ingest_request(&delivery, &records, &CountingIds::default()).unwrap();
    assert_eq!(url, "https://api.galileo.ai/ingest/traces/project-1");
    assert_eq!(traces_of(&payload).len(), 2);
    assert_eq!(
        payload.get("logging_method"),
        Some(&Json::Text("api_direct".to_owned()))
    );
}

#[test]
fn token_auth_ingests_under_v2_projects() {
    let (url, _payload) = ingest_request(
        &delivery(TOKEN_ENV),
        &[record("a")],
        &CountingIds::default(),
    )
    .unwrap();
    assert_eq!(url, "https://galileo.example/v2/projects/project-1/traces");
}

#[test]
fn every_record_takes_a_trace_id_then_a_span_id_from_the_source() {
    let delivery = delivery(API_KEY_ENV);
    let (_url, payload) = ingest_request(
        &delivery,
        &[record("a"), record("b")],
        &CountingIds::default(),
    )
    .unwrap();
    let ids: Vec<String> = traces_of(&payload)
        .iter()
        .filter_map(|trace| match trace {
            Json::Object(fields) => match fields.get("id") {
                Some(Json::Text(id)) => Some(id.clone()),
                _ => None,
            },
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect();
    assert_eq!(ids, ["id-0", "id-2"]);
}

#[test]
fn the_log_stream_id_rides_in_the_payload() {
    let mut variables = API_KEY_ENV.to_vec();
    variables.push(("GALILEO_LOG_STREAM_ID", "stream-9"));
    let (_url, payload) = ingest_request(
        &delivery(&variables),
        &[record("a")],
        &CountingIds::default(),
    )
    .unwrap();
    assert_eq!(
        payload.get("log_stream_id"),
        Some(&Json::Text("stream-9".to_owned()))
    );
}

#[rstest]
#[case::no_project(&[("GALILEO_API_KEY", "key")])]
#[case::no_base_url(&[("GALILEO_USERNAME", "u"), ("GALILEO_PASSWORD", "p"), ("GALILEO_PROJECT_ID", "x")])]
fn without_a_project_and_base_url_there_is_no_ingest_request(#[case] variables: &[(&str, &str)]) {
    let ids = CountingIds::default();
    assert!(ingest_request(&delivery(variables), &[record("a")], &ids).is_none());
    assert_eq!(ids.next_id(), "id-0");
}

#[test]
fn an_empty_buffer_flushes_nothing() {
    let delivery = delivery(API_KEY_ENV);
    let transport = ScriptedTransport::default();
    assert_eq!(
        deliver(&delivery, &transport, &CountingIds::default()),
        FlushOutcome::Empty
    );
    assert!(transport.calls().is_empty());
}

#[test]
fn a_flush_without_a_project_keeps_the_records_and_sends_nothing() {
    let delivery = delivery(&[("GALILEO_API_KEY", "key")]);
    append(&delivery, record("a"));
    let transport = ScriptedTransport::default();
    assert_eq!(
        deliver(&delivery, &transport, &CountingIds::default()),
        FlushOutcome::NotConfigured
    );
    assert!(transport.calls().is_empty());
    assert_eq!(labels(&delivery), ["a"]);
}

#[test]
fn a_flush_that_cannot_log_in_keeps_the_records_and_sends_nothing() {
    let delivery = delivery(TOKEN_ENV);
    append(&delivery, record("a"));
    let transport = ScriptedTransport::replying(vec![Ok(response(401, "{}"))]);
    assert_eq!(
        deliver(&delivery, &transport, &CountingIds::default()),
        FlushOutcome::NoHeaders
    );
    assert_eq!(transport.calls().len(), 1);
    assert_eq!(labels(&delivery), ["a"]);
}

#[test]
fn a_successful_flush_posts_the_payload_and_empties_the_buffer() {
    let delivery = delivery(API_KEY_ENV);
    append(&delivery, record("a"));
    append(&delivery, record("b"));
    let transport = ScriptedTransport::replying(vec![Ok(response(200, "{}"))]);
    assert_eq!(
        deliver(&delivery, &transport, &CountingIds::default()),
        FlushOutcome::Delivered
    );
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(
        calls[0].url,
        "https://api.galileo.ai/ingest/traces/project-1"
    );
    assert!(calls[0].headers.contains(&(
        "Galileo-API-Key".to_owned(),
        "key-0123456789abcdef".to_owned()
    )));
    assert_eq!(traces_of(&calls[0].payload.clone().unwrap()).len(), 2);
    assert!(labels(&delivery).is_empty());
}

#[test]
fn a_successful_flush_removes_only_the_records_it_sent() {
    let shared = Arc::new(delivery(API_KEY_ENV));
    append(&shared, record("a"));
    append(&shared, record("b"));
    let transport = ScriptedTransport {
        append_during_post: Some((Arc::clone(&shared), record("late"))),
        ..ScriptedTransport::replying(vec![Ok(response(204, ""))])
    };
    assert_eq!(
        deliver(&shared, &transport, &CountingIds::default()),
        FlushOutcome::Delivered
    );
    assert_eq!(labels(&shared), ["late"]);
}

#[rstest]
#[case::server_error(500)]
#[case::bad_request(400)]
#[case::redirect(302)]
fn a_non_2xx_flush_keeps_every_record_and_logs_the_status(#[case] status: u16) {
    let (logs, _guard) = capture_logs();
    let delivery = delivery(API_KEY_ENV);
    append(&delivery, record("a"));
    append(&delivery, record("b"));
    let transport = ScriptedTransport::replying(vec![Ok(response(status, "boom body"))]);
    assert_eq!(
        deliver(&delivery, &transport, &CountingIds::default()),
        FlushOutcome::Rejected
    );
    assert_eq!(labels(&delivery), ["a", "b"]);
    let text = logs.text();
    assert!(text.contains(&format!("status code={status}")), "{text}");
    assert!(text.contains("boom body"), "{text}");
}

#[test]
fn an_unreachable_flush_keeps_every_record_and_logs_the_failure() {
    let (logs, _guard) = capture_logs();
    let delivery = delivery(API_KEY_ENV);
    append(&delivery, record("a"));
    let transport = ScriptedTransport::replying(vec![unreachable_host()]);
    assert_eq!(
        deliver(&delivery, &transport, &CountingIds::default()),
        FlushOutcome::TransportFailed
    );
    assert_eq!(labels(&delivery), ["a"]);
    assert!(
        logs.text().contains("failed to flush in memory records"),
        "{}",
        logs.text()
    );
}

#[rstest]
#[case::unauthorized(401)]
#[case::forbidden(403)]
fn token_auth_rejection_drops_the_cached_headers_so_the_next_flush_logs_in(#[case] status: u16) {
    let delivery = delivery(TOKEN_ENV);
    append(&delivery, record("a"));
    let transport = ScriptedTransport::replying(vec![
        Ok(response(200, LOGIN_REPLY)),
        Ok(response(status, "{}")),
        Ok(response(200, LOGIN_REPLY)),
        Ok(response(200, "{}")),
    ]);
    let ids = CountingIds::default();
    assert_eq!(deliver(&delivery, &transport, &ids), FlushOutcome::Rejected);
    assert_eq!(cached_header_names(&delivery), None);
    assert_eq!(labels(&delivery), ["a"]);
    assert_eq!(
        deliver(&delivery, &transport, &ids),
        FlushOutcome::Delivered
    );
    let urls: Vec<String> = transport.calls().into_iter().map(|call| call.url).collect();
    assert_eq!(urls.iter().filter(|url| url.ends_with("/login")).count(), 2);
    assert!(labels(&delivery).is_empty());
}

#[rstest]
#[case::unauthorized(401)]
#[case::forbidden(403)]
fn the_v2_api_key_survives_a_rejection(#[case] status: u16) {
    let delivery = delivery(API_KEY_ENV);
    append(&delivery, record("a"));
    let transport = ScriptedTransport::replying(vec![Ok(response(status, "{}"))]);
    assert_eq!(
        deliver(&delivery, &transport, &CountingIds::default()),
        FlushOutcome::Rejected
    );
    assert!(cached_header_names(&delivery).is_some());
}

#[test]
fn a_server_error_keeps_the_token_headers() {
    let delivery = delivery(TOKEN_ENV);
    append(&delivery, record("a"));
    let transport = ScriptedTransport::replying(vec![
        Ok(response(200, LOGIN_REPLY)),
        Ok(response(500, "{}")),
    ]);
    assert_eq!(
        deliver(&delivery, &transport, &CountingIds::default()),
        FlushOutcome::Rejected
    );
    assert!(cached_header_names(&delivery).is_some());
}

#[test]
fn flush_debug_logs_never_show_the_credential() {
    let (logs, _guard) = capture_logs();
    let delivery = delivery(API_KEY_ENV);
    append(&delivery, record("a"));
    let transport = ScriptedTransport::replying(vec![Ok(response(200, "{}"))]);
    deliver(&delivery, &transport, &CountingIds::default());
    let text = logs.text();
    assert!(text.contains("key-0123...cdef"), "{text}");
    assert!(!text.contains("key-0123456789abcdef"), "{text}");
}
