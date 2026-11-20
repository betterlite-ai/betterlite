#![cfg(test)]

mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::FileObject;
use http::{HeaderMap as HttpHeaders, Method};
use rstest::rstest;
use serde_json::Number;

use self::support::{
    Answer, BASE, Events, FILE_ID, Failure, Fixture, IMPATIENT, PATIENT, ProxiedSource,
    RecordingConfig, Reply, SECRET, SHORT, Source, Stage, TIMEOUT, Wire, caller_headers, deleted,
    file_object, files, litellm_params, litellm_params_with, metadata_deps, run, target,
};
use crate::api::file_http_metadata::config::FileParams;
use crate::api::file_http_metadata::error::MetadataError;
use crate::api::file_http_metadata::service::{
    DeleteCall, ListCall, MetadataCall, MetadataDeps, MetadataTarget, MetadataValue, RetrieveCall,
};
use crate::api::vector_store_http_write::error::HeaderMap;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Attempt {
    RetrieveSync,
    RetrieveAsync,
    DeleteSync,
    DeleteAsync,
    ListSync,
    ListAsync,
}

impl Attempt {
    const fn mode(self) -> ClientKind {
        match self {
            Self::RetrieveSync | Self::DeleteSync | Self::ListSync => ClientKind::Sync,
            Self::RetrieveAsync | Self::DeleteAsync | Self::ListAsync => ClientKind::Asynchronous,
        }
    }

    fn call<'call>(
        self,
        params: &'call FileParams,
        timeout: Option<Duration>,
    ) -> MetadataCall<'call> {
        match self {
            Self::RetrieveSync | Self::RetrieveAsync => MetadataCall::Retrieve(RetrieveCall {
                file_id: FILE_ID,
                target: target(params, None, timeout),
            }),
            Self::DeleteSync | Self::DeleteAsync => MetadataCall::Delete(DeleteCall {
                file_id: FILE_ID,
                target: target(params, None, timeout),
            }),
            Self::ListSync | Self::ListAsync => MetadataCall::List(ListCall {
                purpose: Some("batch"),
                target: target(params, None, timeout),
            }),
        }
    }
}

fn success_answers(count: usize) -> Vec<Answer> {
    (0..count)
        .map(|_| Answer::Reply(Reply::json(200, "{}")))
        .collect()
}

#[tokio::test]
async fn metadata_request_order() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params_with(BASE, SECRET, &[("provider_flag", "1")]);
    let headers = caller_headers(&[("x-trace", "trace-1")]);
    let mut fixture = Fixture::new("file_retrieve");
    let call = MetadataCall::Retrieve(RetrieveCall {
        file_id: FILE_ID,
        target: target(&params, Some(&headers), None),
    });
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the retrieve succeeds");
    assert_eq!(value, MetadataValue::File(file_object(FILE_ID)));
    assert_eq!(
        events.recorded(),
        vec!["request", "validate", "log", "send", "response"]
    );
    let requested = config.requested();
    let transform = requested.first().expect("the request transform runs first");
    assert_eq!(
        transform.litellm_keys,
        vec![
            "api_base".to_owned(),
            "api_key".to_owned(),
            "provider_flag".to_owned()
        ]
    );
    let validated = config.validates();
    let validation = validated.first().expect("the environment is validated");
    assert_eq!(validation.api_key.as_deref(), Some(SECRET));
    assert_eq!(wire.requests()[0].url.query(), Some("view=full"));
}

#[tokio::test]
async fn retrieve_wire_arguments() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let headers = caller_headers(&[("x-trace", "trace-1")]);
    let mut fixture = Fixture::new("file_retrieve");
    let call = MetadataCall::Retrieve(RetrieveCall {
        file_id: FILE_ID,
        target: target(&params, Some(&headers), Some(TIMEOUT)),
    });
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the retrieve succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].url.path(), "/files/file_f");
    assert_eq!(requests[0].url.query(), Some("view=full"));
    assert_eq!(requests[0].timeout, None);
    assert!(matches!(requests[0].body, RequestBody::Empty));
    let sent = |name: &str| {
        requests[0]
            .headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    };
    assert_eq!(sent("authorization"), Some("Bearer sk-provider"));
    assert_eq!(sent("x-validation"), Some("1"));
    assert_eq!(sent("x-trace"), Some("trace-1"));
    assert_eq!(source.calls(), vec![None]);
}

#[tokio::test]
async fn retrieve_call_arguments() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let headers = caller_headers(&[("x-trace", "trace-1")]);
    let mut fixture = Fixture::new("file_retrieve");
    let call = MetadataCall::Retrieve(RetrieveCall {
        file_id: FILE_ID,
        target: target(&params, Some(&headers), None),
    });
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the retrieve succeeds");
    let validated = config.validates();
    let first = validated
        .first()
        .expect("the environment is validated once");
    assert_eq!(validated.len(), 1);
    assert_eq!(first.api_key.as_deref(), Some(SECRET));
    assert_eq!(first.model, "");
    assert_eq!(first.messages, 0);
    assert!(first.optional_empty);
    assert_eq!(first.headers, vec!["x-trace".to_owned()]);
    assert_eq!(first.incoming_validation, None);
    assert_eq!(fixture.input(), Some(&Json::Text(String::new())));
    assert_eq!(
        fixture.logged("file_id"),
        Some(&Json::Text(FILE_ID.to_owned()))
    );
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(format!("{BASE}/files/{FILE_ID}")))
    );
    let requested = config.requested();
    let transform = requested.first().expect("the transform runs once");
    assert_eq!(transform.file_id.as_deref(), Some(FILE_ID));
    assert_eq!(
        transform.litellm_keys,
        vec!["api_base".to_owned(), "api_key".to_owned()]
    );
    let observed = config.observed();
    assert!(
        observed
            .first()
            .expect("the response transform runs")
            .additional_args
    );
}

#[tokio::test]
async fn retrieve_logs_masked_headers() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let headers = caller_headers(&[("x-trace", "trace-1")]);
    let mut fixture = Fixture::new("file_retrieve");
    let call = MetadataCall::Retrieve(RetrieveCall {
        file_id: FILE_ID,
        target: target(&params, Some(&headers), None),
    });
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the retrieve succeeds");
    let Json::Object(logged) = fixture
        .logged("headers")
        .expect("the pre-call logs the validated headers")
    else {
        panic!("the logged headers are an object");
    };
    assert_eq!(
        logged.get("authorization"),
        Some(&Json::Text("Be****er".to_owned()))
    );
    assert_eq!(
        logged.get("x-trace"),
        Some(&Json::Text("trace-1".to_owned()))
    );
}

#[tokio::test]
async fn delete_wire_and_value() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_delete");
    let call = Attempt::DeleteSync.call(&params, Some(TIMEOUT));
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the delete succeeds");
    assert_eq!(value, MetadataValue::Deleted(deleted()));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::DELETE);
    assert_eq!(requests[0].url.path(), "/files/file_f");
    assert_eq!(requests[0].url.query(), Some("view=full"));
    assert_eq!(requests[0].timeout, None);
    assert!(matches!(requests[0].body, RequestBody::Empty));
}

#[tokio::test]
async fn list_arguments() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_pages(vec![vec![file_object("a-0")]]);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_list");
    let call = Attempt::ListSync.call(&params, Some(TIMEOUT));
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the listing succeeds");
    assert_eq!(value, MetadataValue::Listed(vec![file_object("a-0")]));
    let requests = wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::GET);
    assert_eq!(requests[0].url.path(), "/files");
    assert_eq!(requests[0].url.query(), Some("purpose=batch"));
    assert_eq!(requests[0].timeout, Some(TIMEOUT));
    assert_eq!(
        config.requested().first().expect("one transform").purpose,
        Some("batch".to_owned())
    );
    assert_eq!(
        fixture.logged("purpose"),
        Some(&Json::Text("batch".to_owned()))
    );
    assert_eq!(
        fixture.logged("api_base"),
        Some(&Json::Text(format!("{BASE}/files")))
    );
    assert_eq!(config.next_calls(), 1);
}

#[tokio::test]
async fn list_without_purpose() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_pages(vec![Vec::new()]);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_list");
    let call = MetadataCall::List(ListCall {
        purpose: None,
        target: target(&params, None, None),
    });
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the listing succeeds");
    assert_eq!(value, MetadataValue::Listed(Vec::new()));
    assert_eq!(wire.requests()[0].url.query(), None);
    assert_eq!(fixture.logged("purpose"), Some(&Json::Null));
}

#[rstest]
#[case::retrieve_sync(Attempt::RetrieveSync, None)]
#[case::retrieve_async(Attempt::RetrieveAsync, None)]
#[case::delete_sync(Attempt::DeleteSync, None)]
#[case::delete_async(Attempt::DeleteAsync, Some(TIMEOUT))]
#[case::list_sync(Attempt::ListSync, Some(TIMEOUT))]
#[case::list_async(Attempt::ListAsync, Some(TIMEOUT))]
#[tokio::test]
async fn metadata_timeout_asymmetry(#[case] attempt: Attempt, #[case] expected: Option<Duration>) {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_pages(vec![vec![file_object("a-0")]]);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_call");
    let call = attempt.call(&params, Some(TIMEOUT));
    run(&mut fixture, &deps, attempt.mode(), &call, &events)
        .await
        .expect("the call succeeds");
    assert_eq!(
        wire.requests().first().expect("one request").timeout,
        expected
    );
}

#[rstest]
#[case::retrieve_sync(Attempt::RetrieveSync, 500)]
#[case::retrieve_async(Attempt::RetrieveAsync, 500)]
#[case::delete_sync(Attempt::DeleteSync, 408)]
#[case::delete_async(Attempt::DeleteAsync, 500)]
#[case::list_sync(Attempt::ListSync, 500)]
#[case::list_async(Attempt::ListAsync, 500)]
#[tokio::test]
async fn timeout_error_follows_the_operation(#[case] attempt: Attempt, #[case] expected: u16) {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let events = Events::new();
    let base = format!("http://{}/v1", serving.address);
    let config = RecordingConfig::new(&base, &events);
    let source = ProxiedSource::new(IMPATIENT);
    let deps = MetadataDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = litellm_params(&base, SECRET);
    let mut fixture = Fixture::new("file_call");
    let call = attempt.call(&params, Some(TIMEOUT));
    let result = run(&mut fixture, &deps, attempt.mode(), &call, &events).await;
    let Err(MetadataError::Provider(error)) = result else {
        panic!("a timeout is a provider error");
    };
    assert_eq!(error.status_code, expected);
    if expected == 408 {
        assert!(!error.status_code_is_synthesized);
        assert_eq!(
            error.message,
            "litellm.Timeout: Connection timed out after None seconds."
        );
    } else {
        assert!(error.status_code_is_synthesized);
        assert!(error.message.contains("read timeout"));
    }
}

#[rstest]
#[case::retrieve_sync(Attempt::RetrieveSync, true)]
#[case::retrieve_async(Attempt::RetrieveAsync, true)]
#[case::delete_sync(Attempt::DeleteSync, true)]
#[case::delete_async(Attempt::DeleteAsync, false)]
#[case::list_sync(Attempt::ListSync, false)]
#[case::list_async(Attempt::ListAsync, false)]
#[tokio::test]
async fn timeout_omission_uses_the_client_default(
    #[case] attempt: Attempt,
    #[case] succeeds: bool,
) {
    let serving = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(100),
    )
    .expect("the fixture serves");
    let events = Events::new();
    let base = format!("http://{}/v1", serving.address);
    let config = RecordingConfig::new(&base, &events);
    config.set_pages(vec![Vec::new()]);
    let source = ProxiedSource::new(PATIENT);
    let deps = MetadataDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let params = litellm_params(&base, SECRET);
    let mut fixture = Fixture::new("file_call");
    let call = attempt.call(&params, Some(SHORT));
    let result = run(&mut fixture, &deps, attempt.mode(), &call, &events).await;
    if succeeds {
        result.expect("an omitted timeout lets the client's own default outlast the delay");
        return;
    }
    let Err(MetadataError::Provider(error)) = result else {
        panic!("a passed timeout ends the call before the reply arrives");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert!(error.message.contains("read timeout"));
}

#[tokio::test]
async fn eager_file_pages() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_pages(vec![
        files("a", 2),
        vec![file_object("b-0"), file_object("a-0")],
        files("c", 1),
    ]);
    config.set_nexts(vec![
        Some(caller_headers(&[("pagination_token", "t1")])),
        Some(caller_headers(&[("pagination_token", "t2")])),
        None,
    ]);
    let wire = Wire::new(success_answers(3), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let headers = caller_headers(&[("x-trace", "trace-1")]);
    let mut fixture = Fixture::new("file_list");
    let call = MetadataCall::List(ListCall {
        purpose: Some("batch"),
        target: target(&params, Some(&headers), Some(TIMEOUT)),
    });
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the listing succeeds");
    let MetadataValue::Listed(listed) = value else {
        panic!("a listing returns files");
    };
    let ids: Vec<&str> = listed.iter().map(|file| file.id.as_str()).collect();
    assert_eq!(ids, vec!["a-0", "a-1", "b-0", "a-0", "c-0"]);
    assert_eq!(config.next_calls(), 3);
    assert_eq!(config.nexts_remaining(), 0);
    let requests = wire.requests();
    assert_eq!(requests[0].url.query(), Some("purpose=batch"));
    assert_eq!(requests[0].timeout, Some(TIMEOUT));
    assert_eq!(
        fixture.logged("purpose"),
        Some(&Json::Text("batch".to_owned()))
    );
    assert_eq!(fixture.logged("file_id"), None);
}

#[tokio::test]
async fn eager_file_pages_continuations() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_pages(vec![files("a", 1), files("b", 1), files("c", 1)]);
    config.set_nexts(vec![
        Some(caller_headers(&[("pagination_token", "t1")])),
        Some(caller_headers(&[("pagination_token", "t2")])),
        None,
    ]);
    let wire = Wire::new(success_answers(3), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_list");
    let call = Attempt::ListSync.call(&params, Some(TIMEOUT));
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the listing succeeds");
    let requests = wire.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1].url.query(), Some("pagination_token=t1"));
    assert_eq!(requests[2].url.query(), Some("pagination_token=t2"));
    assert_eq!(
        requests
            .iter()
            .filter(|sent| sent.timeout == Some(TIMEOUT))
            .count(),
        3
    );
    let validates = config.validates();
    assert_eq!(validates.len(), 3);
    assert_eq!(validates[1].incoming_validation.as_deref(), Some("1"));
    assert_eq!(validates[2].incoming_validation.as_deref(), Some("1"));
    let observed = config.observed();
    assert_eq!(observed.len(), 3);
    assert_eq!(observed[0].previous_page, None);
    assert_eq!(
        observed[1].previous_page,
        Some(Json::Number(Number::from(0_u32)))
    );
    assert!(observed.iter().all(|entry| entry.additional_args));
}

#[tokio::test]
async fn response_transforms_receive_the_sent_url() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_pages(vec![files("a", 1), files("b", 1)]);
    config.set_nexts(vec![Some(caller_headers(&[("pagination_token", "t1")]))]);
    let wire = Wire::new(success_answers(2), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_list");
    let call = Attempt::ListSync.call(&params, Some(TIMEOUT));
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the listing succeeds");
    let sent: Vec<Option<String>> = wire
        .requests()
        .iter()
        .map(|request| Some(request.url.to_string()))
        .collect();
    assert_eq!(sent.len(), 2);
    assert_eq!(config.urls(), sent);
}

#[tokio::test]
async fn an_empty_page_without_a_continuation_ends_the_listing() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_pages(vec![Vec::new()]);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_list");
    let call = Attempt::ListSync.call(&params, Some(TIMEOUT));
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the listing succeeds");
    assert_eq!(value, MetadataValue::Listed(Vec::new()));
    assert_eq!(wire.requests().len(), 1);
    assert_eq!(config.next_calls(), 1);
}

#[tokio::test]
async fn an_empty_page_with_a_continuation_is_followed() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_pages(vec![Vec::new(), files("b", 1)]);
    config.set_nexts(vec![
        Some(caller_headers(&[("pagination_token", "t1")])),
        None,
    ]);
    let wire = Wire::new(success_answers(2), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_list");
    let call = Attempt::ListSync.call(&params, Some(TIMEOUT));
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the listing succeeds");
    assert_eq!(value, MetadataValue::Listed(files("b", 1)));
    let requests = wire.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].url.query(), Some("pagination_token=t1"));
    assert_eq!(config.next_calls(), 2);
}

#[rstest]
#[case::first_page(false, 1_usize)]
#[case::second_page(true, 2_usize)]
#[tokio::test]
async fn a_transport_failure_mid_listing_returns_no_partial_list(
    #[case] second_page: bool,
    #[case] requests: usize,
) {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let answers = if second_page {
        config.set_pages(vec![files("a", 1), files("b", 1)]);
        config.set_nexts(vec![Some(caller_headers(&[("pagination_token", "t1")]))]);
        vec![
            Answer::Reply(Reply::json(200, "{}")),
            Answer::Fail(TransportError::Closed),
        ]
    } else {
        vec![Answer::Fail(TransportError::Closed)]
    };
    let wire = Wire::new(answers, &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_list");
    let call = Attempt::ListSync.call(&params, Some(TIMEOUT));
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(MetadataError::Provider(error)) = result else {
        panic!("a failed page is an error, never a partial list");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, TransportError::Closed.to_string());
    assert_eq!(wire.requests().len(), requests);
}

#[derive(Debug, Clone, Copy)]
enum CeilingCase {
    ExactlyTheLimit,
    BeyondTheLimit,
    SplitAcrossPages,
}

#[rstest]
#[case::exactly_the_limit(CeilingCase::ExactlyTheLimit)]
#[case::beyond_the_limit(CeilingCase::BeyondTheLimit)]
#[case::split_across_pages(CeilingCase::SplitAcrossPages)]
#[tokio::test]
async fn file_listing_ceiling(#[case] case: CeilingCase) {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let (pages, nexts, expected_len, expected_last, expected_nexts, expected_requests): (
        Vec<Vec<FileObject>>,
        Vec<Option<BTreeMap<String, String>>>,
        usize,
        &str,
        usize,
        usize,
    ) = match case {
        CeilingCase::ExactlyTheLimit => {
            (vec![files("a", 10_000)], Vec::new(), 10_000, "a-9999", 0, 1)
        }
        CeilingCase::BeyondTheLimit => {
            (vec![files("a", 10_001)], Vec::new(), 10_000, "a-9999", 0, 1)
        }
        CeilingCase::SplitAcrossPages => (
            vec![files("a", 9_999), files("b", 3)],
            vec![Some(caller_headers(&[("pagination_token", "t1")]))],
            10_000,
            "b-0",
            1,
            2,
        ),
    };
    config.set_pages(pages);
    config.set_nexts(nexts);
    let wire = Wire::new(success_answers(expected_requests), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_list");
    let call = Attempt::ListSync.call(&params, Some(TIMEOUT));
    let value = run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the listing succeeds");
    let MetadataValue::Listed(listed) = value else {
        panic!("a listing returns files");
    };
    assert_eq!(listed.len(), expected_len);
    assert_eq!(
        listed.last().map(|file| file.id.as_str()),
        Some(expected_last)
    );
    assert_eq!(config.next_calls(), expected_nexts);
    assert_eq!(wire.requests().len(), expected_requests);
}

#[rstest]
#[case::request(Stage::Request)]
#[case::validate(Stage::Validate)]
#[case::response(Stage::Response)]
#[tokio::test]
async fn stage_failures_stop_the_call(#[case] stage: Stage) {
    let events = Events::new();
    let config = RecordingConfig::failing(BASE, Failure::at(stage, 403, "refused"), &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_retrieve");
    let call = Attempt::RetrieveSync.call(&params, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(MetadataError::Provider(error)) = result else {
        panic!("a stage failure is a provider error");
    };
    assert_eq!(error.status_code, 403);
    assert_eq!(error.message, "refused");
    let sent = if stage == Stage::Response { 1 } else { 0 };
    let validated = if stage == Stage::Request { 0 } else { 1 };
    assert_eq!(wire.requests().len(), sent);
    assert_eq!(config.validates().len(), validated);
    assert_eq!(source.calls(), vec![None]);
}

#[tokio::test]
async fn transport_failure_reaches_the_config() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(vec![Answer::Fail(TransportError::Closed)], &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_retrieve");
    let call = Attempt::RetrieveSync.call(&params, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(MetadataError::Provider(error)) = result else {
        panic!("a transport failure is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert_eq!(error.message, TransportError::Closed.to_string());
    assert!(error.headers.is_empty());
    assert_eq!(
        config.errors(),
        vec![(
            TransportError::Closed.to_string(),
            500_u16,
            HeaderMap::new()
        )]
    );
    assert_eq!(events.recorded().last().map(String::as_str), Some("error"));
}

#[rstest]
#[case::sync(Attempt::DeleteSync)]
#[case::asynchronous(Attempt::DeleteAsync)]
#[tokio::test]
async fn delete_status_error_is_masked(#[case] attempt: Attempt) {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(
        vec![Answer::Reply(
            Reply::json(500, "boom")
                .with_header("content-encoding", "gzip")
                .with_header("content-length", "99")
                .with_header("content-type", "text/plain"),
        )],
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_delete");
    let call = attempt.call(&params, None);
    let result = run(&mut fixture, &deps, attempt.mode(), &call, &events).await;
    let Err(MetadataError::Provider(error)) = result else {
        panic!("a 500 is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "boom");
    assert_eq!(
        error.headers.get("content-type"),
        Some(&Json::Text("text/plain".to_owned()))
    );
    assert_eq!(
        error.headers.get("content-length"),
        Some(&Json::Text("4".to_owned()))
    );
    assert_eq!(error.headers.get("content-encoding"), None);
    assert_eq!(
        events.recorded(),
        vec!["request", "validate", "log", "send", "error"]
    );
}

#[tokio::test]
async fn retrieve_errors_reach_the_transform() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(
        vec![Answer::Reply(
            Reply::json(502, "backend")
                .with_header("content-encoding", "gzip")
                .with_header("content-length", "99"),
        )],
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_retrieve");
    let call = Attempt::RetrieveSync.call(&params, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(MetadataError::Provider(error)) = result else {
        panic!("the transform's error is a provider error");
    };
    assert_eq!(error.status_code, 502);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.message, "backend");
    assert_eq!(
        error.headers.get("content-length"),
        Some(&Json::Text("99".to_owned()))
    );
    assert_eq!(
        error.headers.get("content-encoding"),
        Some(&Json::Text("gzip".to_owned()))
    );
    assert!(config.errors().is_empty());
    assert_eq!(
        events.recorded(),
        vec!["request", "validate", "log", "send", "response"]
    );
}

#[rstest]
#[case::delete(true)]
#[case::retrieve(false)]
#[tokio::test]
async fn non_utf8_replies_are_named_by_their_length(#[case] delete: bool) {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(
        vec![Answer::Reply(Reply {
            status: 500,
            headers: HttpHeaders::new(),
            body: vec![0xff, 0xfe, 0xf0],
        })],
        &events,
    );
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_call");
    let call = if delete {
        Attempt::DeleteSync.call(&params, None)
    } else {
        Attempt::RetrieveSync.call(&params, None)
    };
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(MetadataError::Provider(error)) = result else {
        panic!("a 500 is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "3 bytes that are not UTF-8");
}

#[rstest]
#[case::name("bad\nname", "value")]
#[case::value("x-trace", "bad\nvalue")]
#[tokio::test]
async fn invalid_caller_headers_are_reported(#[case] name: &str, #[case] value: &str) {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let headers = caller_headers(&[(name, value)]);
    let mut fixture = Fixture::new("file_retrieve");
    let call = MetadataCall::Retrieve(RetrieveCall {
        file_id: FILE_ID,
        target: target(&params, Some(&headers), None),
    });
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    match result {
        Err(
            MetadataError::HeaderName { name: reported, .. }
            | MetadataError::HeaderValue { name: reported, .. },
        ) => {
            assert_eq!(reported, name);
        }
        _ => panic!("the caller's header is reported"),
    }
    assert_eq!(wire.requests().len(), 0);
}

#[tokio::test]
async fn a_query_value_must_be_text() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    config.set_number_query();
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_retrieve");
    let call = Attempt::RetrieveSync.call(&params, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(MetadataError::QueryValue { name }) = result else {
        panic!("a query value that is not text is reported");
    };
    assert_eq!(name, "view");
    assert_eq!(wire.requests().len(), 0);
}

#[tokio::test]
async fn a_base_that_is_not_a_url_is_reported() {
    let events = Events::new();
    let config = RecordingConfig::new("provider.test", &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_retrieve");
    let call = Attempt::RetrieveSync.call(&params, None);
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(MetadataError::RequestUrl { url, .. }) = result else {
        panic!("a base that is not a URL is reported");
    };
    assert_eq!(url, "provider.test/files/file_f");
    assert_eq!(wire.requests().len(), 0);
}

#[tokio::test]
async fn a_supplied_client_serves_only_its_mode() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let source_wire = Wire::new(success_answers(1), &events);
    let supplied_wire = Wire::new(success_answers(1), &events);
    let source = Source::new(source_wire.clone());
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_retrieve");
    let call = Attempt::RetrieveSync.call(&params, None);
    {
        let supplied = SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &supplied_wire,
        };
        let deps = metadata_deps(&config, &source, Some(supplied));
        run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
            .await
            .expect("the sync retrieve succeeds");
    }
    assert_eq!(source.calls(), vec![None]);
    assert_eq!(source_wire.requests().len(), 1);
    assert_eq!(supplied_wire.requests().len(), 0);
    {
        let supplied = SuppliedClient {
            kind: ClientKind::Asynchronous,
            transport: &supplied_wire,
        };
        let deps = metadata_deps(&config, &source, Some(supplied));
        run(
            &mut fixture,
            &deps,
            ClientKind::Asynchronous,
            &call,
            &events,
        )
        .await
        .expect("the async retrieve succeeds");
    }
    assert_eq!(source.calls(), vec![None]);
    assert_eq!(supplied_wire.requests().len(), 1);
}

#[tokio::test]
async fn the_source_receives_ssl_verify() {
    let events = Events::new();
    let config = RecordingConfig::new(BASE, &events);
    let wire = Wire::new(success_answers(1), &events);
    let source = Source::new(wire.clone());
    let deps = metadata_deps(&config, &source, None);
    let params = litellm_params(BASE, SECRET);
    let mut fixture = Fixture::new("file_retrieve");
    let call = MetadataCall::Retrieve(RetrieveCall {
        file_id: FILE_ID,
        target: MetadataTarget {
            litellm_params: &params,
            extra_headers: None,
            ssl_verify: Some(true),
            timeout: None,
        },
    });
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the retrieve succeeds");
    assert_eq!(source.calls(), vec![Some(true)]);
}
