use std::time::Duration;

use betterlite_http::interactions::base::ClientError;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::controls::json_value::JsonValue;

use super::support::{
    CHARACTER_BODY, FakeWire, Fixture, Mode, async_http_client, blocking_client, create_call,
    get_call, params, real_clients, video,
};
use crate::api::video_character_transport::clients::SuppliedClients;
use crate::api::video_character_transport::error::CharacterError;
use crate::api::video_character_transport::service::{CharacterHandled, CharacterTransport};

const BASE: &str = "https://api.example/v1";

#[test]
fn sync_creation_selects_through_the_sync_factory_with_ssl_verify() {
    let mut fixture = Fixture::new();
    let mut litellm = params(&[("api_base", BASE)]);
    litellm.insert("ssl_verify".to_owned(), JsonValue::Bool(false));
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    fixture.sync_create(&call, None).unwrap();

    assert_eq!(
        fixture.clients.selections(),
        [(Mode::Sync, Some(JsonValue::Bool(false)))]
    );
}

#[tokio::test]
async fn async_creation_selects_through_the_async_factory_with_ssl_verify() {
    let mut fixture = Fixture::new();
    let mut litellm = params(&[("api_base", BASE)]);
    litellm.insert("ssl_verify".to_owned(), JsonValue::Bool(false));
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    fixture.async_create(&call, None).await.unwrap();

    assert_eq!(
        fixture.clients.selections(),
        [(Mode::Asynchronous, Some(JsonValue::Bool(false)))]
    );
}

#[test]
fn sync_lookup_selects_without_ssl_verify_when_the_params_name_none() {
    let mut fixture = Fixture::new();
    let litellm = params(&[("api_base", BASE)]);
    let call = get_call("char-1", &litellm);

    fixture.sync_get(&call, None).unwrap();

    assert_eq!(fixture.clients.selections(), [(Mode::Sync, None)]);
}

#[tokio::test]
async fn async_lookup_selects_with_ssl_verify_from_the_params() {
    let mut fixture = Fixture::new();
    let mut litellm = params(&[("api_base", BASE)]);
    litellm.insert("ssl_verify".to_owned(), JsonValue::Bool(true));
    let call = get_call("char-1", &litellm);

    fixture.async_get(&call, None).await.unwrap();

    assert_eq!(
        fixture.clients.selections(),
        [(Mode::Asynchronous, Some(JsonValue::Bool(true)))]
    );
}

#[test]
fn selection_failure_surfaces_before_the_environment() {
    let mut fixture = Fixture::new();
    fixture.fail_selection(ClientError::FieldType {
        field: "ssl_verify",
        expected: "a boolean",
    });
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    let error = fixture.sync_create(&call, None).unwrap_err();

    let CharacterError::Client(ClientError::FieldType { field, .. }) = error else {
        panic!("the factory's refusal surfaces");
    };
    assert_eq!(field, "ssl_verify");
    assert_eq!(fixture.events.recorded(), ["select-sync"]);
}

#[test]
fn a_supplied_sync_client_sends_instead_of_the_factories() {
    let mut fixture = Fixture::new();
    let events = fixture.events.clone();
    let supplied = FakeWire::new(&events, 200, CHARACTER_BODY);
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    fixture.sync_create(&call, Some(&supplied)).unwrap();

    assert_eq!(supplied.requests().len(), 1);
    assert_eq!(fixture.wire.requests().len(), 0);
}

#[tokio::test]
async fn a_supplied_async_client_sends_instead_of_the_factories() {
    let mut fixture = Fixture::new();
    let events = fixture.events.clone();
    let supplied = FakeWire::new(&events, 200, CHARACTER_BODY);
    let litellm = params(&[("api_base", BASE)]);
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);

    fixture.async_create(&call, Some(&supplied)).await.unwrap();

    assert_eq!(supplied.requests().len(), 1);
    assert_eq!(fixture.wire.requests().len(), 0);
}

#[test]
fn the_real_sync_factory_refuses_an_unusable_ssl_verify() {
    let mut fixture = Fixture::new();
    let mut litellm = params(&[("api_base", BASE)]);
    litellm.insert(
        "ssl_verify".to_owned(),
        JsonValue::Text(String::from("yes")),
    );
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);
    let mut clients = real_clients();
    let env = fixture.log.env();
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut clients,
        &mut fixture.state,
        &env,
    );

    let error = match transport.video_create_character_handler(&call) {
        Ok(_handled) => panic!("a non-boolean ssl_verify refuses the client"),
        Err(error) => error,
    };

    let CharacterError::Client(ClientError::FieldType { field, .. }) = error else {
        panic!("the factory reads ssl_verify from the params");
    };
    assert_eq!(field, "ssl_verify");
}

#[tokio::test]
async fn the_real_async_factory_refuses_an_unusable_ssl_verify() {
    let mut fixture = Fixture::new();
    let mut litellm = params(&[("api_base", BASE)]);
    litellm.insert(
        "ssl_verify".to_owned(),
        JsonValue::Text(String::from("yes")),
    );
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);
    let mut clients = real_clients();
    let env = fixture.log.env();
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut clients,
        &mut fixture.state,
        &env,
    );

    let error = transport
        .async_video_create_character_handler(&call)
        .await
        .unwrap_err();

    let CharacterError::Client(ClientError::FieldType { field, .. }) = error else {
        panic!("the factory reads ssl_verify from the params");
    };
    assert_eq!(field, "ssl_verify");
}

#[test]
fn a_supplied_sync_client_skips_the_factories_ssl_verify_check() {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let mut fixture = Fixture::new();
    fixture.config.create_url = format!("{base}/v1/characters");
    let mut litellm = params(&[("api_base", base.as_str())]);
    litellm.insert(
        "ssl_verify".to_owned(),
        JsonValue::Text(String::from("yes")),
    );
    let supplied = blocking_client();
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);
    let mut clients = real_clients();
    let env = fixture.log.env();
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut clients,
        &mut fixture.state,
        &env,
    )
    .supplying(SuppliedClients {
        sync: Some(&supplied),
        asynchronous: None,
    });

    let object = match transport.video_create_character_handler(&call) {
        Ok(CharacterHandled::Done(object)) => object,
        Ok(CharacterHandled::Pending(_)) => panic!("a sync call answers in place"),
        Err(error) => panic!("the supplied client sends the call: {error:?}"),
    };

    assert_eq!(object, fixture.config.object);
    assert_eq!(
        serving.finish().unwrap().line,
        "POST /v1/characters HTTP/1.1"
    );
}

#[tokio::test]
async fn a_supplied_async_client_skips_the_factories_ssl_verify_check() {
    let serving = serve(reply("200 OK", &[], "{}"), Duration::ZERO).unwrap();
    let base = format!("http://{}", serving.address);
    let mut fixture = Fixture::new();
    fixture.config.create_url = format!("{base}/v1/characters");
    let mut litellm = params(&[("api_base", base.as_str())]);
    litellm.insert(
        "ssl_verify".to_owned(),
        JsonValue::Text(String::from("yes")),
    );
    let supplied = async_http_client();
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);
    let mut clients = real_clients();
    let env = fixture.log.env();
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut clients,
        &mut fixture.state,
        &env,
    )
    .supplying(SuppliedClients {
        sync: None,
        asynchronous: Some(&supplied),
    });

    let object = transport
        .async_video_create_character_handler(&call)
        .await
        .unwrap();

    assert_eq!(object, fixture.config.object);
    assert_eq!(
        serving.finish().unwrap().line,
        "POST /v1/characters HTTP/1.1"
    );
}

#[tokio::test]
async fn the_sync_handler_ignores_an_async_only_supplied_client() {
    let mut fixture = Fixture::new();
    let supplied = async_http_client();
    let mut litellm = params(&[("api_base", BASE)]);
    litellm.insert(
        "ssl_verify".to_owned(),
        JsonValue::Text(String::from("yes")),
    );
    let clip = video();
    let call = create_call("Wren", &clip, &litellm);
    let mut clients = real_clients();
    let env = fixture.log.env();
    let mut transport = CharacterTransport::with_generated_boundary(
        &fixture.config,
        &mut clients,
        &mut fixture.state,
        &env,
    )
    .supplying(SuppliedClients {
        sync: None,
        asynchronous: Some(&supplied),
    });

    let error = match transport.video_create_character_handler(&call) {
        Ok(_handled) => panic!("the async client is not the class a sync call sends with"),
        Err(error) => error,
    };

    let CharacterError::Client(ClientError::FieldType { field, .. }) = error else {
        panic!("the sync factory still runs");
    };
    assert_eq!(field, "ssl_verify");
}
