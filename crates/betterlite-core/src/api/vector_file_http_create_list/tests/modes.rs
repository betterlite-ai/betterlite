use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;

use super::support::{
    CallRequest, Events, Fixture, IMPATIENT, ProxiedSource, RecordingConfig, Reply, Source, Wire,
    file_params, openai_config, openai_settings, params,
};
use super::{FILE, run};
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_file_http_create_list::service::{
    ClientKind, CreateCall, FileCall, FileDeps, ListCall,
};

#[tokio::test]
async fn create_passes_timeout_in_both_modes() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let files = params("http://mock.test/v1");
    let timed = CallRequest {
        timeout: Some(Duration::from_secs(7)),
        ..CallRequest::new("vs_a", &files)
    };
    let plain = CallRequest::new("vs_a", &files);
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let mut fixture = Fixture::new("vector_store_file_create");
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        for request in [&timed, &plain] {
            let call = FileCall::Create(CreateCall {
                target: request.target(),
                create_request: &original,
                extra_body: None,
            });
            run(&mut fixture, &deps, mode, &call, &events)
                .await
                .expect("the create succeeds");
        }
    }
    let requests = wire.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(7)));
    assert_eq!(requests[1].timeout, None);
    assert_eq!(requests[2].timeout, Some(Duration::from_secs(7)));
    assert_eq!(requests[3].timeout, None);
}

#[tokio::test]
async fn list_never_passes_timeout() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-provider");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, FILE), &events);
    let source = Source::new(wire.clone());
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let files = params("http://mock.test/v1");
    let timed = CallRequest {
        timeout: Some(Duration::from_secs(7)),
        ..CallRequest::new("vs_a", &files)
    };
    let plain = CallRequest::new("vs_a", &files);
    let query = file_params(r#"{"limit":2}"#);
    let mut fixture = Fixture::new("vector_store_file_list");
    for mode in [ClientKind::Sync, ClientKind::Asynchronous] {
        for request in [&timed, &plain] {
            let call = FileCall::List(ListCall {
                target: request.target(),
                query_params: &query,
                extra_query: None,
            });
            run(&mut fixture, &deps, mode, &call, &events)
                .await
                .expect("the list succeeds");
        }
    }
    let requests = wire.requests();
    assert_eq!(requests.len(), 4);
    for sent in requests.iter() {
        assert_eq!(sent.timeout, None);
    }
}

#[tokio::test]
async fn create_timeout_messages_follow_the_mode() {
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let files = params("http://unused.test");
    let sync_slow = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let sync_config = RecordingConfig::new(&format!(
        "http://{}/v1/vector_stores/vs_a/files",
        sync_slow.address
    ));
    let events = Events::new();
    let source = ProxiedSource::new(IMPATIENT);
    let deps = FileDeps {
        config: &sync_config,
        source: &source,
        supplied: None,
    };
    let request = CallRequest {
        timeout: Some(Duration::from_millis(100)),
        ..CallRequest::new("vs_a", &files)
    };
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = FileCall::Create(CreateCall {
        target: request.target(),
        create_request: &original,
        extra_body: None,
    });
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("a sync timeout is a provider error");
    };
    assert_eq!(error.status_code, 408);
    assert!(!error.status_code_is_synthesized);
    assert_eq!(
        error.message,
        "litellm.Timeout: Connection timed out after 0.1 seconds."
    );
    let async_slow = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let async_config = RecordingConfig::new(&format!(
        "http://{}/v1/vector_stores/vs_a/files",
        async_slow.address
    ));
    let async_deps = FileDeps {
        config: &async_config,
        source: &source,
        supplied: None,
    };
    let async_result = run(
        &mut fixture,
        &async_deps,
        ClientKind::Asynchronous,
        &call,
        &events,
    )
    .await;
    let Err(FileTransportError::Provider(async_error)) = async_result else {
        panic!("an async timeout is a provider error");
    };
    assert_eq!(async_error.status_code, 408);
    assert!(
        async_error
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=0.1, time taken=")
    );
    assert!(async_error.message.ends_with(" seconds"));
}

#[tokio::test]
async fn create_timeout_without_a_value_names_none() {
    let original = file_params(r#"{"file_id":"file_a"}"#);
    let files = params("http://unused.test");
    let slow = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let config = RecordingConfig::new(&format!(
        "http://{}/v1/vector_stores/vs_a/files",
        slow.address
    ));
    let events = Events::new();
    let source = ProxiedSource::new(IMPATIENT);
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    let request = CallRequest::new("vs_a", &files);
    let mut fixture = Fixture::new("vector_store_file_create");
    let call = FileCall::Create(CreateCall {
        target: request.target(),
        create_request: &original,
        extra_body: None,
    });
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("a sync timeout is a provider error");
    };
    assert_eq!(error.status_code, 408);
    assert_eq!(
        error.message,
        "litellm.Timeout: Connection timed out after None seconds."
    );
    // no per-call timeout on the async side either, so the pooled client's read timeout is the only deadline
    let slow_async = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let blank_config = RecordingConfig::new(&format!(
        "http://{}/v1/vector_stores/vs_a/files",
        slow_async.address
    ));
    let blank_deps = FileDeps {
        config: &blank_config,
        source: &source,
        supplied: None,
    };
    let blank_result = run(
        &mut fixture,
        &blank_deps,
        ClientKind::Asynchronous,
        &call,
        &events,
    )
    .await;
    let Err(FileTransportError::Provider(blank_error)) = blank_result else {
        panic!("an async timeout is a provider error");
    };
    assert_eq!(blank_error.status_code, 408);
    assert!(
        blank_error
            .message
            .starts_with("litellm.Timeout: Connection timed out. Timeout passed=None, time taken=")
    );
}

#[tokio::test]
async fn list_timeout_is_a_synthesized_500() {
    let query = file_params(r#"{"limit":2}"#);
    let files = params("http://unused.test");
    let slow = serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves");
    let config = RecordingConfig::new(&format!(
        "http://{}/v1/vector_stores/vs_a/files",
        slow.address
    ));
    let events = Events::new();
    let source = ProxiedSource::new(IMPATIENT);
    let deps = FileDeps {
        config: &config,
        source: &source,
        supplied: None,
    };
    // the timeout argument is seven seconds, so a list that passed it would wait for the slow answer
    let request = CallRequest {
        timeout: Some(Duration::from_secs(7)),
        ..CallRequest::new("vs_a", &files)
    };
    let mut fixture = Fixture::new("vector_store_file_list");
    let call = FileCall::List(ListCall {
        target: request.target(),
        query_params: &query,
        extra_query: None,
    });
    let result = run(&mut fixture, &deps, ClientKind::Sync, &call, &events).await;
    let Err(FileTransportError::Provider(error)) = result else {
        panic!("a list timeout is a provider error");
    };
    assert_eq!(error.status_code, 500);
    assert!(error.status_code_is_synthesized);
    assert!(error.message.contains("read timeout"));
    assert!(error.headers.is_empty());
}
