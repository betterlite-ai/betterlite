use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::super::service::Operation;
use super::support::{
    API_BASE, Fixture, Query, REQUEST_TIMEOUT, Spy, VECTOR_STORE_ID, Which, call, record, recorded,
};

#[rstest]
#[case::sync(Which::Retrieve)]
#[case::asynchronous(Which::Aretrieve)]
#[tokio::test]
async fn retrieve_preserves_record(#[case] which: Which) {
    let spy = Spy::replying(record());
    let mut fixture = Fixture::new();
    fixture.arguments.extra_query = Some(IndexMap::from([(
        String::from("after"),
        Json::Text(String::from("ignored")),
    )]));
    fixture.arguments.extra_body = Some(IndexMap::from([(
        String::from("name"),
        Json::Text(String::from("ignored")),
    )]));
    fixture.params.api_key = Some(SecretString::from("sk-caller"));
    let value = call(which, &spy, &fixture, REQUEST_TIMEOUT)
        .await
        .expect("retrieve answers the record");
    assert_eq!(value, record());
    let seen = spy.seen().expect("the service saw the read");
    assert_eq!(seen.operation, Operation::Retrieve);
    assert_eq!(seen.mode, which.mode());
    assert_eq!(seen.provider, ProviderId::Openai);
    assert_eq!(seen.vector_store_id.as_deref(), Some(VECTOR_STORE_ID));
    assert_eq!(seen.query, Query::default());
    // the caller's key is the one the transport would send, and no other source is consulted
    assert_eq!(seen.api_base.as_deref(), Some(API_BASE));
    assert_eq!(seen.api_key.as_deref(), Some("sk-caller"));
    // model=None leaves the fixture's model, and the id alone is the optional parameter
    assert_eq!(seen.model, "vector-store");
    assert_eq!(
        seen.optional,
        IndexMap::from([(
            String::from("vector_store_id"),
            Json::Text(String::from(VECTOR_STORE_ID)),
        )])
    );
    assert_eq!(
        seen.litellm_params,
        IndexMap::from([(String::from("litellm_call_id"), Json::Null)])
    );
    assert_eq!(seen.provider_name.as_deref(), Some("openai"));
    assert_eq!(
        seen.details.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert_eq!(spy.call_count(), 1);
    assert_eq!(fixture.configs.seen().len(), 1);
}

// the async façade sets the flag itself, so a caller's non-true aretrieve does not change the handler
#[tokio::test]
async fn aretrieve_ignores_a_caller_flag() {
    let spy = Spy::replying(record());
    let mut fixture = Fixture::new();
    fixture
        .arguments
        .kwargs
        .insert(String::from("aretrieve"), Json::Bool(false));
    let seen = recorded(Which::Aretrieve, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(seen.mode, Which::Aretrieve.mode());
    assert_eq!(seen.litellm_params.get("aretrieve"), None);
}
