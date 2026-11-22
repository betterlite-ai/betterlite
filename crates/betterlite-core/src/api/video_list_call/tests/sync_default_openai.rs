#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;

use crate::api::video_list_call::{VideoListCall, VideoListReply};
use crate::api::video_remix_call::support::{
    Bench, Case, LIST_BODY, Plan, Wire, list_call, request_header, secret,
};
use crate::api::video_remix_list_transport::client::VideoClient;

const REQUEST_TIMEOUT: f64 = 6000.0;

#[test]
fn video_list_defaults_to_openai() {
    let mut bench = Bench::new(200, LIST_BODY);
    let reply = bench
        .list_answer(Plan::seconds(
            Case::List(&VideoListCall::default()),
            REQUEST_TIMEOUT,
        ))
        .expect("the list answers");
    let VideoListReply::Page(page) = reply else {
        panic!("a provider answer is a page");
    };
    assert!(page.contains_key("data"));
    assert!(bench.source.sync_calls().is_empty());
    assert_eq!(bench.source.async_calls(), vec![(ProviderId::Openai, None)]);
    let requests = bench.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, http::Method::GET);
    assert_eq!(requests[0].url.as_str(), "https://api.openai.com/v1/videos");
    assert_eq!(requests[0].timeout, None);
}

#[test]
fn video_list_defaults_the_authorization_header_to_bearer_none() {
    let mut bench = Bench::new(200, LIST_BODY);
    bench
        .list_answer(Plan::seconds(
            Case::List(&VideoListCall::default()),
            REQUEST_TIMEOUT,
        ))
        .expect("the list answers");
    let requests = bench.requests();
    assert_eq!(request_header(&requests[0], "Authorization"), "Bearer None");
}

#[test]
fn video_list_reads_the_key_from_the_params() {
    let mut bench = Bench::new(200, LIST_BODY);
    let mut call = list_call();
    call.api_key = Some(secret("sk-typed"));
    bench
        .list_answer(Plan::seconds(Case::List(&call), REQUEST_TIMEOUT))
        .expect("the list answers");
    let requests = bench.requests();
    assert_eq!(
        request_header(&requests[0], "Authorization"),
        "Bearer sk-typed"
    );
}

#[test]
fn video_list_uses_a_supplied_async_client() {
    let mut bench = Bench::new(200, LIST_BODY);
    let wire = Wire::replying(200, LIST_BODY);
    let reply = bench
        .list_answer(Plan::client(
            Case::List(&VideoListCall::default()),
            VideoClient::Asynchronous(&wire),
        ))
        .expect("the supplied client answers");
    assert!(matches!(reply, VideoListReply::Page(_)));
    assert_eq!(wire.requests().len(), 1);
    assert!(bench.requests().is_empty());
    assert!(bench.source.async_calls().is_empty());
}
