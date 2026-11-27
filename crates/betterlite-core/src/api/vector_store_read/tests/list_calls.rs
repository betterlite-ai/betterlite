use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use serde_json::Number;

use super::super::service::Operation;
use super::support::{Fixture, Query, REQUEST_TIMEOUT, Spy, Which, call, listing, recorded};
use crate::api::vector_store_http_write::service::ClientKind;

#[tokio::test]
async fn list_defaults_and_cursors() {
    // no arguments: the signature's own defaults reach the provider
    let spy = Spy::replying(listing());
    let fixture = Fixture::new();
    let value = call(Which::List, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect("the list answers");
    assert_eq!(value, listing());
    let seen = spy.seen().expect("the service saw the read");
    assert_eq!(seen.operation, Operation::List);
    assert_eq!(seen.mode, ClientKind::Sync);
    assert_eq!(seen.provider, ProviderId::Openai);
    assert_eq!(seen.vector_store_id, None);
    assert_eq!(
        seen.query,
        Query {
            after: None,
            before: None,
            limit: Some(20),
            order: Some(String::from("desc")),
        }
    );
    assert_eq!(
        seen.optional,
        IndexMap::from([
            (String::from("after"), Json::Null),
            (String::from("before"), Json::Null),
            (String::from("limit"), Json::Number(Number::from(20_i64))),
            (String::from("order"), Json::Text(String::from("desc"))),
        ])
    );
    let mut fixture = Fixture::new();
    fixture.arguments.after = Some(String::new());
    fixture.arguments.before = Some(String::from("vs_z"));
    fixture.arguments.limit = Some(Some(0_i64));
    fixture.arguments.order = Some(Some(String::from("asc")));
    let spy = Spy::replying(listing());
    let seen = recorded(Which::Alist, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.mode, ClientKind::Asynchronous);
    assert_eq!(
        seen.query,
        Query {
            after: Some(String::new()),
            before: Some(String::from("vs_z")),
            limit: Some(0),
            order: Some(String::from("asc")),
        }
    );
    assert_eq!(
        seen.optional,
        IndexMap::from([
            (String::from("after"), Json::Text(String::new())),
            (String::from("before"), Json::Text(String::from("vs_z"))),
            (String::from("limit"), Json::Number(Number::from(0_i64))),
            (String::from("order"), Json::Text(String::from("asc"))),
        ])
    );
    // an explicit null leaves the query and stays a null in the log
    fixture.arguments.limit = Some(None);
    fixture.arguments.order = Some(None);
    let spy = Spy::replying(listing());
    let seen = recorded(Which::List, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(
        seen.query,
        Query {
            after: Some(String::new()),
            before: Some(String::from("vs_z")),
            limit: None,
            order: None,
        }
    );
    assert_eq!(seen.optional.get("limit"), Some(&Json::Null));
    assert_eq!(seen.optional.get("order"), Some(&Json::Null));
    // an unsupported order is forwarded without local validation, and an absent limit takes the default
    fixture.arguments.limit = None;
    fixture.arguments.order = Some(Some(String::from("sideways")));
    let spy = Spy::replying(listing());
    let seen = recorded(Which::List, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.query.order.as_deref(), Some("sideways"));
    assert_eq!(
        seen.optional.get("order"),
        Some(&Json::Text(String::from("sideways")))
    );
    assert_eq!(
        seen.optional.get("limit"),
        Some(&Json::Number(Number::from(20_i64)))
    );
}
