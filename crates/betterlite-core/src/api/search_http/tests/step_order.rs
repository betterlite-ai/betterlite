use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::pool::ClientPool;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::search::provider::SearchQuery;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::ANSWER;
use super::support::{
    Fixture, TestProvider, default_tls, proxy_policy, recorded, search_call, seen_values,
    test_provider,
};
use crate::api::search_http::SearchHttpCall;

struct SignedRun {
    fixture: Fixture,
    provider: TestProvider,
    served: Serving,
    base: String,
}

async fn signed_search() -> SignedRun {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = TestProvider {
        signed: BTreeMap::from([("x-signature".to_owned(), "abc".to_owned())]),
        ..test_provider(&base)
    };
    let query = SearchQuery::Single(String::from("rust"));
    let params = Object::new();
    let credential = SecretString::from("sk-search");
    let mut fixture = Fixture::new();
    let call = SearchHttpCall {
        api_key: Some(&credential),
        headers: BTreeMap::from([("x-caller".to_owned(), "1".to_owned())]),
        ..search_call(&query, &params, &base, &tls, &routing)
    };
    fixture
        .execute(call, &provider, &pool, false)
        .await
        .expect("the search succeeds");
    SignedRun {
        fixture,
        provider,
        served,
        base,
    }
}

#[tokio::test]
async fn hooks_run_in_order_and_see_the_call() {
    let run = signed_search().await;
    assert_eq!(
        recorded(&run.provider.events),
        vec!["validate", "transform", "url", "sign"]
    );
    let validated = BTreeMap::from([
        ("x-caller".to_owned(), "1".to_owned()),
        ("x-validated".to_owned(), "yes".to_owned()),
    ]);
    let hooks = seen_values(&run.provider);
    assert_eq!(hooks.validate_api_key.as_deref(), Some("sk-search"));
    assert_eq!(hooks.validate_api_base.as_deref(), Some(run.base.as_str()));
    assert_eq!(
        hooks.validate_headers,
        BTreeMap::from([("x-caller".to_owned(), "1".to_owned())])
    );
    assert_eq!(hooks.transform_api_key.as_deref(), Some("sk-search"));
    assert_eq!(hooks.transform_api_base.as_deref(), Some(run.base.as_str()));
    assert_eq!(hooks.transform_headers, validated);
    assert_eq!(hooks.url_api_base.as_deref(), Some(run.base.as_str()));
    assert_eq!(hooks.url_api_key.as_deref(), Some("sk-search"));
    assert_eq!(hooks.url_params, Object::new());
    assert_eq!(hooks.url_body.as_ref(), Some(&run.provider.body));
    assert_eq!(hooks.sign_api_base, run.base);
    assert_eq!(hooks.sign_api_key.as_deref(), Some("sk-search"));
    assert_eq!(hooks.sign_headers, validated);
    assert_eq!(hooks.sign_body, br#"{"q":"rust"}"#.to_vec());
    assert_eq!(hooks.sign_params, Object::new());
}

#[tokio::test]
async fn pre_call_logs_the_unsigned_call() {
    let run = signed_search().await;
    assert_eq!(run.fixture.input(), Some(&Json::Text(String::from("rust"))));
    assert_eq!(
        run.fixture.logged("api_base"),
        Some(&Json::Text(String::from(run.base.as_str())))
    );
    assert_eq!(
        run.fixture.logged("complete_input_dict"),
        Some(&Json::Object(IndexMap::from([(
            "q".to_owned(),
            Json::Text(String::from("rust")),
        )])))
    );
    assert_eq!(
        run.fixture.logged("headers"),
        Some(&Json::Object(IndexMap::from([
            ("x-caller".to_owned(), Json::Text(String::from("1"))),
            ("x-validated".to_owned(), Json::Text(String::from("yes"))),
        ])))
    );
    assert_eq!(
        run.served.finish().expect("the fixture answers").line,
        "POST /search HTTP/1.1"
    );
}

#[tokio::test]
async fn list_query_logs_as_json_text() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}/search", served.address);
    let tls = default_tls();
    let routing = proxy_policy();
    let pool = ClientPool::new(4, TransportLimits::DEFAULT);
    let provider = test_provider(&base);
    let query = SearchQuery::Multiple(vec![String::from("rust"), String::from("search")]);
    let params = Object::new();
    let mut fixture = Fixture::new();
    fixture
        .execute(
            search_call(&query, &params, &base, &tls, &routing),
            &provider,
            &pool,
            false,
        )
        .await
        .expect("the list query succeeds");
    assert_eq!(
        fixture.input(),
        Some(&Json::Text(String::from(r#"["rust","search"]"#)))
    );
    served.finish().expect("the fixture answers");
}
