use betterlite_http::interactions::base::ClientError;

use super::support::{
    CHARACTER_BODY, Events, FakeClients, FakeWire, Fixture, LogEnv, create_call, get_call,
    logging_state, params, recording, video,
};
use crate::api::video_character_transport::service::{CharacterHandled, CharacterTransport};

const BASE: &str = "https://api.example/v1";

#[tokio::test]
async fn sync_creation_dispatches_to_the_async_handler_when_asked() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.is_async = true;
    let env = fixture.log.env();
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );

    let answer = transport.video_create_character_handler(&call).unwrap();

    assert!(fixture.events.recorded().is_empty());
    let CharacterHandled::Pending(pending) = answer else {
        panic!("the async flag returns the pending call");
    };
    let object = pending.await.unwrap();
    assert_eq!(object, fixture.config.object);
    assert_eq!(
        fixture.events.recorded(),
        [
            "select-async",
            "validate",
            "url",
            "transform-create",
            "log",
            "send",
            "transform-create-response",
        ]
    );
}

#[tokio::test]
async fn sync_lookup_dispatches_to_the_async_handler_when_asked() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let mut call = get_call("char-1", &litellm);
    call.is_async = true;
    let env = fixture.log.env();
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut fixture.clients,
        &mut fixture.state,
        &env,
    );

    let answer = transport.video_get_character_handler(&call).unwrap();

    assert!(fixture.events.recorded().is_empty());
    let CharacterHandled::Pending(pending) = answer else {
        panic!("the async flag returns the pending call");
    };
    let object = pending.await.unwrap();
    assert_eq!(object, fixture.config.object);
    assert_eq!(
        fixture.events.recorded(),
        [
            "select-async",
            "validate",
            "url",
            "transform-get",
            "log",
            "send",
            "transform-get-response",
        ]
    );
}

#[test]
fn creation_dispatches_before_selecting_a_client() {
    let events = Events::new();
    let wire = FakeWire::new(&events, 200, CHARACTER_BODY);
    let mut clients = FakeClients::failing(
        &events,
        wire,
        ClientError::FieldType {
            field: "ssl_verify",
            expected: "a boolean",
        },
    );
    let config = recording(&events);
    let log = LogEnv::for_events(&events);
    let mut state = logging_state("video_create_character");
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let mut call = create_call("Wren", &clip, &litellm);
    call.is_async = true;
    let env = log.env();
    let mut transport =
        CharacterTransport::with_generated_boundary(&config, &mut clients, &mut state, &env);

    let answer = transport.video_create_character_handler(&call).unwrap();

    assert!(matches!(answer, CharacterHandled::Pending(_)));
    assert!(events.recorded().is_empty());
}

#[tokio::test]
async fn async_creation_handler_runs_the_whole_flow() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let object = fixture.async_create(&call, None).await.unwrap();

    assert_eq!(object, fixture.config.object);
    assert_eq!(
        fixture.events.recorded(),
        [
            "select-async",
            "validate",
            "url",
            "transform-create",
            "log",
            "send",
            "transform-create-response",
        ]
    );
    assert_eq!(fixture.wire.requests().len(), 1);
}

#[tokio::test]
async fn async_lookup_handler_runs_the_whole_flow() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let call = get_call("char-1", &litellm);

    let object = fixture.async_get(&call, None).await.unwrap();

    assert_eq!(object, fixture.config.object);
    assert_eq!(
        fixture.events.recorded(),
        [
            "select-async",
            "validate",
            "url",
            "transform-get",
            "log",
            "send",
            "transform-get-response",
        ]
    );
    assert_eq!(fixture.wire.requests().len(), 1);
}
