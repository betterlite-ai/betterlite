use axum::http::{Method, StatusCode};
use betterlite_db::identity_rows::Payload;
use betterlite_db::resource_rows::credential::CredentialItem;

use super::support::{
    ErrorReply, FakeLookup, FakeStore, ListReply, call, credential, payload, registry, sealed,
    state, text,
};

#[tokio::test]
async fn get_reads_memory_and_not_the_store() {
    let registry = registry(vec![credential(
        "openai",
        &[],
        &[("api_key", text("sk-memory"))],
    )]);
    let store = FakeStore::seeded(vec![credential(
        "openai",
        &[],
        &[("api_key", text(&sealed("sk-store")))],
    )]);
    let reply = call(
        state(&registry, Some(&store), None),
        Method::GET,
        "/credentials",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let listed: ListReply = reply.json();
    assert_eq!(1, listed.credentials.len());
    assert_eq!(
        payload(&[("api_key", text("sk****ry"))]),
        listed.credentials[0].fields.credential_values
    );
}

#[tokio::test]
async fn the_list_masks_each_credentials_values() {
    let registry = registry(vec![
        credential(
            "openai",
            &[("team", text("ml"))],
            &[
                ("api_key", text("sk-abcdefgh")),
                ("api_base", text("https://a")),
            ],
        ),
        credential("short", &[], &[("token", text("abcd"))]),
    ]);
    let reply = call(
        state(&registry, None, None),
        Method::GET,
        "/credentials",
        "",
        None,
    )
    .await;
    let listed: ListReply = reply.json();
    assert!(listed.success);
    assert_eq!(2, listed.credentials.len());
    assert_eq!(
        payload(&[("team", text("ml"))]),
        listed.credentials[0].fields.base.credential_info
    );
    assert_eq!(
        payload(&[
            ("api_key", text("sk****gh")),
            ("api_base", text("https://a")),
        ]),
        listed.credentials[0].fields.credential_values
    );
    assert_eq!("short", listed.credentials[1].fields.base.credential_name);
    assert_eq!(
        payload(&[("token", text("*****"))]),
        listed.credentials[1].fields.credential_values
    );
}

#[tokio::test]
async fn by_name_masks_the_first_match() {
    let registry = registry(vec![
        credential("openai", &[], &[("api_key", text("sk-first"))]),
        credential("openai", &[], &[("api_key", text("sk-second"))]),
    ]);
    let reply = call(
        state(&registry, None, None),
        Method::GET,
        "/credentials/by_name/openai",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let item: CredentialItem = reply.json();
    assert_eq!("openai", item.fields.base.credential_name);
    assert_eq!(
        payload(&[("api_key", text("sk****st"))]),
        item.fields.credential_values
    );
}

#[tokio::test]
async fn by_name_of_a_missing_credential_is_a_404_with_its_name() {
    let registry = registry(Vec::new());
    let reply = call(
        state(&registry, None, None),
        Method::GET,
        "/credentials/by_name/missing",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::NOT_FOUND, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!(
        "Credential not found. Got credential name: missing",
        body.error.message
    );
    assert_eq!("404", body.error.code);
    assert_eq!("internal_server_error", body.error.error_type);
}

#[tokio::test]
async fn by_name_reads_a_name_with_slashes() {
    let registry = registry(vec![credential(
        "azure/prod",
        &[],
        &[("api_key", text("sk-abcdefgh"))],
    )]);
    let reply = call(
        state(&registry, None, None),
        Method::GET,
        "/credentials/by_name/azure/prod",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let item: CredentialItem = reply.json();
    assert_eq!("azure/prod", item.fields.base.credential_name);
    assert_eq!(
        payload(&[("api_key", text("sk****gh"))]),
        item.fields.credential_values
    );
}

#[tokio::test]
async fn by_model_names_the_deployment_credential_and_masks_the_values() {
    let registry = registry(Vec::new());
    let lookup = FakeLookup::serving("dep-1", "gpt-4o", &[("api_key", text("sk-abcdefgh"))]);
    let reply = call(
        state(&registry, None, Some(&lookup)),
        Method::GET,
        "/credentials/by_model/dep-1",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::OK, reply.status);
    let item: CredentialItem = reply.json();
    assert_eq!("gpt-4o-credential-dep-1", item.fields.base.credential_name);
    assert_eq!(Payload::default(), item.fields.base.credential_info);
    assert_eq!(
        payload(&[("api_key", text("sk****gh"))]),
        item.fields.credential_values
    );
}

#[tokio::test]
async fn by_model_without_a_router_is_a_500() {
    let registry = registry(Vec::new());
    let reply = call(
        state(&registry, None, None),
        Method::GET,
        "/credentials/by_model/dep-1",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::INTERNAL_SERVER_ERROR, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("LLM router not found", body.error.message);
    assert_eq!("500", body.error.code);
}

#[tokio::test]
async fn by_model_of_an_unknown_model_is_a_404() {
    let registry = registry(Vec::new());
    let lookup = FakeLookup::serving("other", "gpt-4o", &[("api_key", text("sk-abcdefgh"))]);
    let reply = call(
        state(&registry, None, Some(&lookup)),
        Method::GET,
        "/credentials/by_model/dep-1",
        "",
        None,
    )
    .await;
    assert_eq!(StatusCode::NOT_FOUND, reply.status);
    let body: ErrorReply = reply.json();
    assert_eq!("Model not found", body.error.message);
    assert_eq!("404", body.error.code);
}
