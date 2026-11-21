#![cfg(test)]

use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::SearchOptions;
use super::params::{call_kwargs, merged_params};
use super::support::{ANSWER, Call, Harness, Recorder, Registry, json, kwargs, object, request};

fn max_results(count: i64) -> SearchOptions {
    SearchOptions {
        max_results: Some(count),
        search_domain_filter: None,
        max_tokens_per_page: None,
        country: None,
    }
}

#[test]
fn kwargs_precedence_keeps_the_explicit_option() {
    let params = merged_params(
        &max_results(5_i64),
        &kwargs(r#"{"max_results":9,"safesearch":"strict"}"#),
    );
    assert_eq!(
        Json::Object(params),
        json(r#"{"max_results":5,"safesearch":"strict"}"#)
    );
}

#[test]
fn kwargs_precedence_fills_an_unset_option() {
    let params = merged_params(&SearchOptions::default(), &kwargs(r#"{"max_results":9}"#));
    assert_eq!(Json::Object(params), json(r#"{"max_results":9}"#));
}

#[test]
fn kwargs_precedence_appends_provider_kwargs_after_the_options() {
    let params = merged_params(&max_results(5_i64), &kwargs(r#"{"safesearch":"strict"}"#));
    assert_eq!(
        params.keys().collect::<Vec<_>>(),
        ["max_results", "safesearch"]
    );
}

#[rstest]
#[case::call_id("litellm_call_id")]
#[case::metadata("metadata")]
#[case::api_key("api_key")]
#[case::internal_prefix("_litellm_marker")]
fn kwargs_precedence_drops_litellm_owned_kwargs(#[case] name: &str) {
    let mut owned = kwargs(r#"{"safesearch":"strict"}"#);
    owned.insert(name.to_owned(), Json::Text("value".to_owned()));
    let params = merged_params(&SearchOptions::default(), &owned);
    assert_eq!(Json::Object(params), json(r#"{"safesearch":"strict"}"#));
}

#[rstest]
#[case::logging_object("litellm_logging_obj")]
#[case::async_flag("asearch")]
fn kwargs_precedence_pops_the_call_flags(#[case] name: &str) {
    let mut flagged = kwargs(r#"{"safesearch":"strict"}"#);
    flagged.insert(name.to_owned(), Json::Bool(true));
    let kept = call_kwargs(&flagged);
    assert_eq!(kept, kwargs(r#"{"safesearch":"strict"}"#));
    assert_eq!(
        Json::Object(merged_params(&SearchOptions::default(), &kept)),
        json(r#"{"safesearch":"strict"}"#)
    );
}

#[tokio::test]
async fn kwargs_precedence_reaches_the_provider() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let recorder = Recorder::default();
    let registry = Registry::new(recorder.clone());
    let mut search = request(r#""rust""#, &format!("http://{}", served.address));
    search.options = max_results(5_i64);
    search.kwargs = kwargs(r#"{"max_results":9,"safesearch":"strict","asearch":true}"#);
    let mut harness = Harness::new();
    harness.search(&search, &registry).await.unwrap();
    let Some(Call::Transform { params, .. }) = recorder
        .calls()
        .into_iter()
        .find(|call| matches!(call, Call::Transform { .. }))
    else {
        panic!("the provider built no request");
    };
    assert_eq!(params, object(r#"{"max_results":5,"safesearch":"strict"}"#));
    served.finish().expect("the fixture answers");
}
