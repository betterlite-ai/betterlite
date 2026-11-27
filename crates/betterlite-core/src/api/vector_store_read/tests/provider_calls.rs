use std::collections::BTreeMap;
use std::collections::BTreeSet;

use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_error::envelope::WireField;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use serde_json::Number;

use super::super::error::VectorStoreReadFailure;
use super::support::{
    Configs, FakeClient, Fixture, REQUEST_TIMEOUT, SeenProvider, Spy, Which, call,
    call_with_client, record, recorded, supplied,
};
use crate::api::vector_store_http_write::service::ClientKind;

fn provider_fixture(provider: &str) -> Fixture {
    let mut fixture = Fixture::new();
    fixture.arguments.provider = Some(String::from(provider));
    fixture
}

// the configuration source serves vertex so the api_type after the slash is observable
fn vertex_fixture(provider: &str) -> Fixture {
    let mut fixture =
        Fixture::with_configs(Configs::serving(BTreeSet::from([ProviderId::VertexAi])));
    fixture.arguments.provider = Some(String::from(provider));
    fixture
}

// exception_type maps a local ValueError to APIConnectionError with its 500 and no wire fields
fn assert_connection_failure(failure: &VectorStoreReadFailure) {
    let raised = failure.raised();
    assert_eq!(raised.class, "APIConnectionError");
    assert_eq!(raised.status, 500_u16);
    assert_eq!(raised.kind, WireField::Null);
    assert_eq!(raised.param, WireField::Null);
    assert_eq!(raised.code, None);
}

fn assert_not_found(failure: &VectorStoreReadFailure) {
    let raised = failure.raised();
    assert_eq!(raised.class, "NotFoundError");
    assert_eq!(raised.status, 404_u16);
    assert_eq!(raised.kind, WireField::Null);
    assert_eq!(raised.param, WireField::Null);
    assert_eq!(raised.code, None);
}

#[tokio::test]
async fn read_provider_failures() {
    // a null provider is openai, and the configuration source is asked for exactly that
    let spy = Spy::replying(record());
    let fixture = Fixture::new();
    let default = recorded(Which::Retrieve, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(default.provider, ProviderId::Openai);
    assert_eq!(
        fixture.configs.seen(),
        vec![SeenProvider {
            provider: ProviderId::Openai,
            api_type: None,
            tag: VectorStoresConfigTag::OpenAI,
        }]
    );

    // an empty or unknown name fails enum resolution before any HTTP or configuration
    for (name, message) in [
        ("", " is not a valid LlmProviders"),
        ("bogus", "bogus is not a valid LlmProviders"),
    ] {
        let refused = Spy::replying(record());
        let fixture = provider_fixture(name);
        let unknown = call(Which::Retrieve, &refused, &fixture, REQUEST_TIMEOUT)
            .await
            .expect_err("the name is not an enum member");
        assert_eq!(unknown.error.to_string(), message);
        assert_connection_failure(&unknown);
        assert_eq!(refused.call_count(), 0);
        assert!(fixture.configs.seen().is_empty());
    }

    // a known provider with no vector store config is refused with the operation's word
    let refused = Spy::replying(record());
    let unserved = Fixture::with_configs(Configs::serving(BTreeSet::new()));
    let retrieve = call(Which::Retrieve, &refused, &unserved, REQUEST_TIMEOUT)
        .await
        .expect_err("the configuration source serves no openai");
    assert_eq!(
        retrieve.error.to_string(),
        "Vector store retrieve is not supported for openai"
    );
    assert_connection_failure(&retrieve);
    let list = call(Which::List, &refused, &unserved, REQUEST_TIMEOUT)
        .await
        .expect_err("the configuration source serves no openai");
    assert_eq!(
        list.error.to_string(),
        "Vector store list is not supported for openai"
    );
    assert_connection_failure(&list);
    assert_eq!(refused.call_count(), 0);
    assert_eq!(unserved.configs.seen().len(), 2);

    // anthropic resolves but serves neither read
    let refused = Spy::replying(record());
    let retrieve = call(
        Which::Retrieve,
        &refused,
        &provider_fixture("anthropic"),
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("anthropic serves no vector store retrieve");
    assert_eq!(
        retrieve.error.to_string(),
        "Vector store retrieve is not supported for anthropic"
    );
    assert_connection_failure(&retrieve);
    let list = call(
        Which::List,
        &refused,
        &provider_fixture("anthropic"),
        REQUEST_TIMEOUT,
    )
    .await
    .expect_err("anthropic serves no vector store list");
    assert_eq!(
        list.error.to_string(),
        "Vector store list is not supported for anthropic"
    );
    assert_connection_failure(&list);
    assert_eq!(refused.call_count(), 0);

    // vertex_ai/rag_api splits into the provider and the api_type the configuration reads
    let spy = Spy::replying(record());
    let fixture = vertex_fixture("vertex_ai/rag_api");
    let rag = recorded(Which::Retrieve, &spy, &fixture, REQUEST_TIMEOUT).await;
    assert_eq!(rag.provider, ProviderId::VertexAi);
    assert_eq!(rag.provider_name.as_deref(), Some("vertex_ai"));
    assert_eq!(
        fixture.configs.seen(),
        vec![SeenProvider {
            provider: ProviderId::VertexAi,
            api_type: Some(String::from("rag_api")),
            tag: VectorStoresConfigTag::Vertex,
        }]
    );

    // a provider error keeps its status in every mode, so a 404 is a NotFoundError
    for which in [Which::Retrieve, Which::Aretrieve, Which::List, Which::Alist] {
        let failing = Spy::failing(404, "the store is gone");
        let fixture = Fixture::new();
        let error = call(which, &failing, &fixture, REQUEST_TIMEOUT)
            .await
            .expect_err("a 404 is an error");
        assert_eq!(error.error.to_string(), "the store is gone");
        assert_not_found(&error);
        assert_eq!(failing.call_count(), 1);
    }

    // the caller's supplied client and the request context reach the provider's transport
    let client = FakeClient { name: "supplied" };
    for (which, kind) in [
        (Which::Retrieve, ClientKind::Sync),
        (Which::Aretrieve, ClientKind::Asynchronous),
    ] {
        let spy = Spy::replying(record());
        let mut fixture = Fixture::new();
        fixture.arguments.extra_headers = Some(BTreeMap::from([(
            String::from("X-Request"),
            String::from("yes"),
        )]));
        let value = call_with_client(
            which,
            &spy,
            &fixture,
            Some(supplied(kind, &client)),
            REQUEST_TIMEOUT,
        )
        .await
        .expect("the read answers");
        assert_eq!(value, record());
        let seen = spy.seen().expect("the service saw the read");
        assert_eq!(seen.client, Some((kind, String::from("supplied"))));
        assert_eq!(
            seen.extra_headers,
            Some(BTreeMap::from([(
                String::from("X-Request"),
                String::from("yes"),
            )]))
        );
        assert_eq!(seen.model, "vector-store");
        assert_eq!(seen.call_type, "endpoint_func");
    }

    // only a boolean true selects the async handler, and the flag never reaches the log
    for flag in ["aretrieve", "alist"] {
        for (value, expected) in [
            (Json::Bool(true), ClientKind::Asynchronous),
            (Json::Number(Number::from(1_u32)), ClientKind::Sync),
            (Json::Text(String::from("true")), ClientKind::Sync),
        ] {
            let spy = Spy::replying(record());
            let mut fixture = Fixture::new();
            fixture.arguments.kwargs.insert(String::from(flag), value);
            let which = if flag == "aretrieve" {
                Which::Retrieve
            } else {
                Which::List
            };
            let seen = recorded(which, &spy, &fixture, REQUEST_TIMEOUT).await;
            assert_eq!(seen.mode, expected);
            assert_eq!(seen.litellm_params.get(flag), None);
        }
    }
}
