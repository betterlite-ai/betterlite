use betterlite_openai::vector_stores::store::CreateParams;
use rstest::rstest;

use super::run;
use super::support::{
    Events, Fixture, Reply, Source, Wire, deps, openai_config, openai_settings, params, target,
};
use crate::api::vector_store_http_create_read::service::{CreateCall, CreateReadCall};
use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

const STORE: &str = r#"{"id":"vs_a","object":"vector_store"}"#;

const fn create_call<'call>(
    params: &'call WriteParams,
    optional: &'call CreateParams,
) -> CreateReadCall<'call> {
    CreateReadCall::Create(CreateCall {
        target: target(params),
        optional_params: optional,
        extra_body: None,
    })
}

// LiteLLM takes the client it was handed only when its class matches the mode; otherwise it asks the provider's pool
#[tokio::test]
async fn supplied_client_serves_only_its_own_mode() {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire.clone());
    let supplied = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &wire,
    };
    let deps = deps(&config, &source, Some(supplied));
    let params = params("http://mock.test/v1");
    let optional = CreateParams::default();
    let mut fixture = Fixture::new("vector_store_create");
    let call = create_call(&params, &optional);
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the supplied client serves the sync create");
    assert!(source.calls().is_empty());
    run(
        &mut fixture,
        &deps,
        ClientKind::Asynchronous,
        &call,
        &events,
    )
    .await
    .expect("a pool client serves the async create");
    assert_eq!(source.calls(), vec![None]);
    assert_eq!(wire.requests().len(), 2);
}

#[rstest]
#[case::unset(None)]
#[case::refused(Some(false))]
#[tokio::test]
async fn pool_client_receives_the_ssl_verify_setting(#[case] ssl_verify: Option<bool>) {
    let settings = openai_settings();
    let config = openai_config(&settings, "sk-test");
    let events = Events::new();
    let wire = Wire::replying(Reply::json(200, STORE), &events);
    let source = Source::new(wire);
    let deps = deps(&config, &source, None);
    let mut configured = params("http://mock.test/v1");
    configured.ssl_verify = ssl_verify;
    let optional = CreateParams::default();
    let mut fixture = Fixture::new("vector_store_create");
    let call = create_call(&configured, &optional);
    run(&mut fixture, &deps, ClientKind::Sync, &call, &events)
        .await
        .expect("the create succeeds");
    assert_eq!(source.calls(), vec![ssl_verify]);
}
