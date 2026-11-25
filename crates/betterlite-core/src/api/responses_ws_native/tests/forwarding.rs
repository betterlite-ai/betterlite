use std::sync::Arc;

use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use secrecy::SecretString;

use crate::api::responses_ws_native::call::{NativeWsCall, NativeWsDeps, responses_websocket};

use super::support::{
    Event, FakeConfigs, FakeResolution, RecordingHandler, RecordingLogging, ScriptedBackend,
    TestClient, lock,
};
use super::texts;

const FIRST_FRAME: &str = r#"{"type":"response.create","input":"hello"}"#;

struct ForwardingFixture {
    backend: Arc<ScriptedBackend>,
    client: TestClient,
    logging: Arc<RecordingLogging>,
    handler: RecordingHandler,
    kwargs: Event,
}

fn forwarding_fixture() -> ForwardingFixture {
    let backend = Arc::new(ScriptedBackend::new());
    backend
        .text(r#"{"type":"response.created"}"#)
        .text(r#"{"type":"response.completed"}"#)
        .closing();
    let mut client = TestClient::new(Vec::new());
    client.yield_first = true;
    let logging = Arc::new(RecordingLogging::default());
    let handler = RecordingHandler::forwarding(Arc::clone(&backend), Arc::clone(&logging));
    let mut kwargs = Event::new();
    kwargs.insert(
        String::from("first_message"),
        Json::Text(FIRST_FRAME.to_owned()),
    );
    ForwardingFixture {
        backend,
        client,
        logging,
        handler,
        kwargs,
    }
}

async fn run_entry(fixture: &ForwardingFixture) -> Option<ErrorClass> {
    let resolution = FakeResolution::openai();
    let configs = FakeConfigs::default();
    let globals = LiteLlmGlobals::default();
    let openai = SecretString::from("sk-env");
    let deps = NativeWsDeps {
        resolution: &resolution,
        configs: &configs,
        logging: &*fixture.logging,
        globals: &globals,
        openai_api_key: Some(&openai),
        auto_summary: false,
        handler: &fixture.handler,
    };
    let call = NativeWsCall {
        model: "gpt-4o",
        websocket: &fixture.client,
        api_base: None,
        api_key: None,
        timeout: Some(7.0_f64),
        user_api_key_dict: None,
        kwargs: &fixture.kwargs,
    };
    responses_websocket(&deps, call).await.unwrap()
}

fn assert_forwarded_call(fixture: &ForwardingFixture) {
    let request = fixture.handler.request(0);
    assert_eq!(request.model, "gpt-4o");
    assert_eq!(request.api_key.as_deref(), Some("sk-env"));
    assert_eq!(request.api_base, None);
    assert_eq!(request.timeout, Some(7.0_f64));
    assert_eq!(request.first_message.as_deref(), Some(FIRST_FRAME));
    assert_eq!(request.custom_llm_provider.as_deref(), Some("openai"));
    assert_eq!(fixture.backend.sent(), texts(&[FIRST_FRAME]));
    assert_eq!(
        fixture.client.sent(),
        texts(&[
            r#"{"type":"response.created"}"#,
            r#"{"type":"response.completed"}"#
        ])
    );
    assert_eq!(
        fixture.logging.effects(),
        texts(&["update", "pre_call", "input_messages", "success"])
    );
    let updates = lock(&fixture.logging.updates).clone();
    assert_eq!(updates[0].model, "gpt-4o");
    assert_eq!(updates[0].provider.as_deref(), Some("openai"));
    assert_eq!(updates[0].user, None);
    assert_eq!(
        updates[0].litellm_params.get("data_residency"),
        Some(&Json::Null)
    );
    assert!(updates[0].kwargs.contains_key("first_message"));
}

async fn forwarding_failure_is_returned() {
    let backend = Arc::new(ScriptedBackend::new());
    backend
        .text(
            r#"{"type":"response.failed","response":{"error":{"type":"server_error","message":"upstream broke"}}}"#,
        )
        .closing();
    let mut client = TestClient::new(Vec::new());
    client.yield_first = true;
    let logging = Arc::new(RecordingLogging::default());
    let handler = RecordingHandler::forwarding(Arc::clone(&backend), Arc::clone(&logging));
    let mut kwargs = Event::new();
    kwargs.insert(
        String::from("first_message"),
        Json::Text(FIRST_FRAME.to_owned()),
    );
    let fixture = ForwardingFixture {
        backend,
        client,
        logging,
        handler,
        kwargs,
    };
    let outcome = run_entry(&fixture).await;
    assert_eq!(
        outcome
            .expect("the stored failure comes back")
            .metadata()
            .status_code,
        500
    );
}

pub(super) async fn native_ws_response_create() {
    let fixture = forwarding_fixture();
    let outcome = run_entry(&fixture).await;
    assert!(outcome.is_none());
    assert_forwarded_call(&fixture);
    forwarding_failure_is_returned().await;
}
