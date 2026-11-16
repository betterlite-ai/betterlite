#![cfg(test)]

#[path = "tests/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::fs;
use std::future::{Ready, ready};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchStatus;
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use jiff::tz::TimeZone;
use rstest::rstest;
use tempfile::NamedTempFile;
use tokio::time::timeout;
use tracing::subscriber::set_default;

use super::BatchLifecycleRequest;
use crate::provider_metadata::timestamp::LocalTimeContext;
use crate::token_cache::error::CredentialCacheError;
use crate::token_cache::probe::AccessToken;
use crate::vertex_batch_lifecycle::cancel::{execute_batch_cancel, prepare_batch_cancel};
use crate::vertex_batch_lifecycle::error::VertexBatchLifecycleError;
use crate::vertex_batch_lifecycle::list::{execute_batch_list, prepare_batch_list};
use crate::vertex_batch_lifecycle::retrieve::{execute_batch_retrieve, prepare_batch_retrieve};
use crate::vertex_batch_records::response::BatchListObject;
use support::{
    CANCELLING, JOB, Reply, SECOND_JOB, Scripted, Stalled, access, hanging_server, page, reply,
    reply_bytes,
};

const HOST: &str =
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1";

fn base<'request>(
    api_base: Option<&'request str>,
    timeout: Option<Duration>,
) -> BatchLifecycleRequest<'request> {
    BatchLifecycleRequest {
        api_base,
        project: Some("proj"),
        location: Some("us-central1"),
        timeout,
        max_retries: None,
    }
}

fn local() -> LocalTimeContext {
    LocalTimeContext {
        zone: TimeZone::get("UTC").unwrap(),
    }
}

fn missing_token() -> Ready<Result<AccessToken, CredentialCacheError>> {
    ready(Err(CredentialCacheError::MissingToken))
}

#[tokio::test]
async fn encoded_read_id() {
    let transport = Scripted::new(vec![reply(200, JOB)]);
    let request = base(None, None);
    let prepared = prepare_batch_retrieve(&request, "job:one/two", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let target = format!("{HOST}/batchPredictionJobs/job%3Aone%2Ftwo");
    assert_eq!(prepared.logging.api_base.as_deref(), Some(target.as_str()));
    let object = execute_batch_retrieve(&transport, prepared, &local())
        .await
        .unwrap();
    assert_eq!(object.id, "777");
    assert_eq!(object.status, BatchStatus::Validating);
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].protected);
    assert_eq!(seen[0].url, target);
}

#[tokio::test]
async fn protected_read_target() {
    let transport = Scripted::new(vec![reply(200, JOB)]);
    let request = base(None, Some(Duration::from_secs(5)));
    let prepared = prepare_batch_retrieve(&request, "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    execute_batch_retrieve(&transport, prepared, &local())
        .await
        .unwrap();
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    let call = &seen[0];
    assert!(call.protected);
    assert_eq!(call.method, Method::GET);
    assert_eq!(call.url, format!("{HOST}/batchPredictionJobs/777"));
    assert_eq!(call.deadline, Duration::from_secs(600));
    let token = call.headers.get(AUTHORIZATION).unwrap();
    assert_eq!(token.to_str().unwrap(), "Bearer tok-123");
    assert!(token.is_sensitive());
    assert_eq!(
        call.headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json; charset=utf-8"
    );
}

#[rstest]
#[case::given(
    Some("p1"),
    Some("europe-west4"),
    "https://europe-west4-aiplatform.googleapis.com/v1/projects/p1/locations/europe-west4/batchPredictionJobs/777"
)]
#[case::absent(
    None,
    None,
    "https://us-central1-aiplatform.googleapis.com/v1/projects/tokproj/locations/us-central1/batchPredictionJobs/777"
)]
#[case::empty(
    Some(""),
    Some(""),
    "https://us-central1-aiplatform.googleapis.com/v1/projects/tokproj/locations/us-central1/batchPredictionJobs/777"
)]
#[tokio::test]
async fn token_project_and_default_location(
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
    #[case] expected: &str,
) {
    let transport = Scripted::new(vec![reply(200, JOB)]);
    let request = BatchLifecycleRequest {
        project,
        location,
        ..base(None, None)
    };
    let prepared = prepare_batch_retrieve(&request, "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    execute_batch_retrieve(&transport, prepared, &local())
        .await
        .unwrap();
    assert_eq!(transport.requests()[0].url, expected);
}

#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::bad_request(400)]
#[case::forbidden(403)]
#[case::server_error(500)]
#[tokio::test]
async fn retrieve_status_is_preserved(#[case] status: u16) {
    let transport = Scripted::new(vec![reply(status, "no job here")]);
    let prepared = prepare_batch_retrieve(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_retrieve(&transport, prepared, &local())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::RetrieveStatus { .. }
    ));
    assert_eq!(error.status_code(), status);
    assert!(error.to_string().contains("no job here"));
}

#[tokio::test]
async fn an_error_body_keeps_the_text_before_its_first_invalid_byte() {
    let transport = Scripted::new(vec![reply_bytes(500, b"bad \xff job")]);
    let prepared = prepare_batch_retrieve(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_retrieve(&transport, prepared, &local())
        .await
        .unwrap_err();
    assert!(error.to_string().ends_with(": bad "));
}

#[tokio::test]
async fn retrieve_logging_masks_the_bearer_token() {
    let request = base(None, None);
    let prepared = prepare_batch_retrieve(&request, "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let logging = prepared.logging;
    let target = format!("{HOST}/batchPredictionJobs/777");
    assert_eq!(logging.api_base.as_deref(), Some(target.as_str()));
    let headers = logging.headers.as_ref().unwrap();
    assert_eq!(
        headers.get("Authorization"),
        Some(&Json::Text("Bearer tok-123".to_owned()))
    );
    assert_eq!(
        headers.get("Content-Type"),
        Some(&Json::Text("application/json; charset=utf-8".to_owned()))
    );
    assert_eq!(logging.complete_input_dict, Some(RawBody::default()));
    let curl = logging.request_str.as_deref().unwrap();
    let first_lines = format!("curl -X GET \\\n{target} \\\n");
    assert!(curl.contains(first_lines.as_str()));
    assert!(curl.contains("-H 'Authorization: Bearer ***REDACTED***'"));
    assert!(!curl.contains("tok-123"));
    let recorded = logging.recorded();
    let Json::Object(fields) = recorded else {
        panic!("the recorded args are an object");
    };
    let Json::Object(recorded_headers) = fields.get("headers").unwrap() else {
        panic!("the recorded headers are an object");
    };
    assert_eq!(
        recorded_headers.get("Authorization"),
        Some(&Json::Text("Be****23".to_owned()))
    );
    assert_eq!(
        recorded_headers.get("Content-Type"),
        Some(&Json::Text("application/json; charset=utf-8".to_owned()))
    );
}

#[rstest]
#[case::absent(None, None, Vec::new())]
#[case::zero_limit(Some(0), None, vec![("pageSize".to_owned(), "0".to_owned())])]
#[case::empty_after(None, Some(""), vec![("pageToken".to_owned(), String::new())])]
#[case::both(
    Some(25),
    Some("page-2"),
    vec![
        ("pageSize".to_owned(), "25".to_owned()),
        ("pageToken".to_owned(), "page-2".to_owned())
    ]
)]
#[tokio::test]
async fn zero_limit_empty_token(
    #[case] limit: Option<i64>,
    #[case] after: Option<&str>,
    #[case] query: Vec<(String, String)>,
) {
    let body = page(&[JOB], "");
    let transport = Scripted::new(vec![reply(200, &body)]);
    let request = base(None, None);
    let prepared = prepare_batch_list(&request, after, limit, ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    execute_batch_list(&transport, prepared, &local())
        .await
        .unwrap();
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    let call = &seen[0];
    assert!(!call.protected);
    assert_eq!(call.method, Method::GET);
    assert_eq!(call.url, format!("{HOST}/batchPredictionJobs"));
    assert_eq!(call.query, query);
    assert_eq!(call.deadline, Duration::from_secs(600));
}

#[tokio::test]
async fn single_page_only() {
    let body = page(&[JOB, SECOND_JOB], "page-2");
    let transport = Scripted::new(vec![reply(200, &body)]);
    let prepared = prepare_batch_list(&base(None, None), None, None, ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let list = execute_batch_list(&transport, prepared, &local())
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 1);
    assert!(matches!(list.object, BatchListObject::List));
    assert!(list.has_more);
    assert_eq!(list.next_page_token.as_deref(), Some("page-2"));
    assert_eq!(list.data.len(), 2);
    assert_eq!(list.first_id.as_deref(), Some("777"));
    assert_eq!(list.last_id.as_deref(), Some("888"));
}

#[rstest]
#[case::created(201)]
#[case::bad_request(400)]
#[case::rate_limited(429)]
#[case::server_error(503)]
#[tokio::test]
async fn list_status_is_preserved(#[case] status: u16) {
    let transport = Scripted::new(vec![reply(status, "no page here")]);
    let prepared = prepare_batch_list(&base(None, None), None, None, ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_list(&transport, prepared, &local())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::ListStatus { .. }
    ));
    assert_eq!(error.status_code(), status);
    assert!(error.to_string().contains("no page here"));
}

#[tokio::test]
async fn list_transport_failure_is_a_500() {
    let transport = Scripted::new(vec![Reply::Failure]);
    let prepared = prepare_batch_list(&base(None, None), None, None, ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_list(&transport, prepared, &local())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::ListTransport { .. }
    ));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn retrieve_transport_failure_is_a_500() {
    let transport = Scripted::new(vec![Reply::Failure]);
    let prepared = prepare_batch_retrieve(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_retrieve(&transport, prepared, &local())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::ProtectedGet { .. }
    ));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn cancel_then_retrieve() {
    let transport = Scripted::new(vec![reply(200, ""), reply(200, CANCELLING)]);
    let request = base(None, Some(Duration::from_secs(5)));
    let prepared = prepare_batch_cancel(&request, "job:one/two", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let object = execute_batch_cancel(&transport, prepared, &local())
        .await
        .unwrap();
    assert_eq!(object.id, "777");
    assert_eq!(object.status, BatchStatus::Cancelling);
    let seen = transport.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, Method::POST);
    assert_eq!(
        seen[0].url,
        format!("{HOST}/batchPredictionJobs/job%3Aone%2Ftwo:cancel")
    );
    assert_eq!(seen[0].body, b"{}".to_vec());
    assert_eq!(
        seen[0].headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json; charset=utf-8"
    );
    assert_eq!(
        seen[0]
            .headers
            .get(AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer tok-123"
    );
    assert_eq!(seen[1].method, Method::GET);
    assert_eq!(
        seen[1].url,
        format!("{HOST}/batchPredictionJobs/job%3Aone%2Ftwo")
    );
    assert!(seen[1].body.is_empty());
    assert!(seen[1].query.is_empty());
}

// raise_for_status passes every 2xx answer of the cancel POST on to the follow-up GET
#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::last_success(299)]
#[tokio::test]
async fn cancel_post_2xx_goes_on_to_the_get(#[case] status: u16) {
    let transport = Scripted::new(vec![reply(status, ""), reply(200, CANCELLING)]);
    let prepared = prepare_batch_cancel(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let object = execute_batch_cancel(&transport, prepared, &local())
        .await
        .unwrap();
    assert_eq!(object.id, "777");
    assert_eq!(object.status, BatchStatus::Cancelling);
    let seen = transport.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].method, Method::POST);
    assert_eq!(seen[1].method, Method::GET);
}

// an unfollowed 3xx fails the POST and aborts the GET
#[rstest]
#[case::multiple_choices(300)]
#[case::not_modified(304)]
#[tokio::test]
async fn cancel_post_outside_2xx_aborts_the_get(#[case] status: u16) {
    let transport = Scripted::new(vec![reply(status, "moved")]);
    let prepared = prepare_batch_cancel(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_cancel(&transport, prepared, &local())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::CancelStatus { .. }
    ));
    assert_eq!(error.status_code(), status);
    assert_eq!(transport.requests().len(), 1);
}

#[tokio::test]
async fn cancel_failure_no_get() {
    let body = "y".repeat(1500);
    let transport = Scripted::new(vec![reply(400, &body)]);
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let prepared = prepare_batch_cancel(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_cancel(&transport, prepared, &local())
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(
        error,
        VertexBatchLifecycleError::CancelStatus { .. }
    ));
    assert_eq!(error.status_code(), 400);
    let seen = transport.requests();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::POST);
    let logged = fs::read_to_string(log.path()).unwrap();
    assert!(logged.contains("Vertex AI batch cancel failed: status=400"));
    assert!(logged.contains(&"y".repeat(1000)));
    assert!(!logged.contains(&"y".repeat(1001)));
}

#[tokio::test]
async fn cancel_timeout_only() {
    let transport = Scripted::new(vec![reply(200, ""), reply(200, CANCELLING)]);
    let request = BatchLifecycleRequest {
        timeout: Some(Duration::from_secs(7)),
        max_retries: Some(9),
        ..base(None, None)
    };
    let prepared = prepare_batch_cancel(&request, "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    execute_batch_cancel(&transport, prepared, &local())
        .await
        .unwrap();
    let deadlines: Vec<Duration> = transport
        .requests()
        .iter()
        .map(|call| call.deadline)
        .collect();
    assert_eq!(
        deadlines,
        vec![Duration::from_secs(7), Duration::from_secs(600)]
    );
}

#[tokio::test]
async fn cancel_timeout_is_a_408() {
    let server = hanging_server().await;
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("a proxy policy"),
    );
    let client =
        ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).expect("a client");
    let base_url = server.base.clone();
    let request = BatchLifecycleRequest {
        api_base: Some(base_url.as_str()),
        timeout: Some(Duration::from_millis(200)),
        ..base(None, None)
    };
    let prepared = prepare_batch_cancel(&request, "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = timeout(
        Duration::from_secs(5),
        execute_batch_cancel(&client, prepared, &local()),
    )
    .await
    .expect("the cancel POST ends on its own deadline")
    .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::CancelTimeout { .. }
    ));
    assert_eq!(error.status_code(), 408);
}

#[tokio::test]
async fn cancel_transport_failure_is_a_500() {
    let transport = Scripted::new(vec![Reply::Failure]);
    let prepared = prepare_batch_cancel(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_cancel(&transport, prepared, &local())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        VertexBatchLifecycleError::CancelTransport { .. }
    ));
    assert_eq!(error.status_code(), 500);
}

#[tokio::test]
async fn retrieve_after_cancel_status_is_preserved() {
    let body = "z".repeat(1500);
    let transport = Scripted::new(vec![reply(200, ""), reply(502, &body)]);
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let guard = set_default(subscriber);
    let prepared = prepare_batch_cancel(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let error = execute_batch_cancel(&transport, prepared, &local())
        .await
        .unwrap_err();
    drop(guard);
    assert!(matches!(
        error,
        VertexBatchLifecycleError::RetrieveAfterCancelStatus { .. }
    ));
    assert_eq!(error.status_code(), 502);
    assert_eq!(transport.requests().len(), 2);
    let logged = fs::read_to_string(log.path()).unwrap();
    assert!(logged.contains("retrieve-after-cancel failed: status=502"));
    assert!(logged.contains(&"z".repeat(1000)));
    assert!(!logged.contains(&"z".repeat(1001)));
}

#[tokio::test]
async fn caller_cancel_closes_response() {
    let transport = Stalled::new();
    let prepared = prepare_batch_cancel(&base(None, None), "777", ready(Ok(access("tokproj"))))
        .await
        .unwrap();
    let outcome = timeout(
        Duration::from_millis(100),
        execute_batch_cancel(&transport, prepared, &local()),
    )
    .await;
    assert!(outcome.is_err());
    assert!(transport.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn credentials_failure_stops_before_any_request() {
    let request = base(None, None);
    let retrieve = prepare_batch_retrieve(&request, "777", missing_token()).await;
    let list = prepare_batch_list(&request, None, None, missing_token()).await;
    let cancel = prepare_batch_cancel(&request, "777", missing_token()).await;
    let retrieve = retrieve.unwrap_err();
    assert!(matches!(
        retrieve,
        VertexBatchLifecycleError::Credentials { .. }
    ));
    assert_eq!(retrieve.status_code(), 500);
    let list = list.unwrap_err();
    assert!(matches!(
        list,
        VertexBatchLifecycleError::Credentials { .. }
    ));
    assert_eq!(list.status_code(), 500);
    let cancel = cancel.unwrap_err();
    assert!(matches!(
        cancel,
        VertexBatchLifecycleError::Credentials { .. }
    ));
    assert_eq!(cancel.status_code(), 500);
}
