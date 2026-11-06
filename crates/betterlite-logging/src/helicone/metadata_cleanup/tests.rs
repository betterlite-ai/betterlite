#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::resolve;
use crate::helicone::event::{LitellmParams, ProxyServerRequest};

fn params(proxy_server_request: Option<ProxyServerRequest>) -> LitellmParams {
    LitellmParams {
        custom_llm_provider: String::new(),
        metadata: IndexMap::from([
            (
                "litellm_parent_otel_span".to_owned(),
                Json::Text("span".to_owned()),
            ),
            ("keep".to_owned(), Json::Bool(true)),
        ]),
        proxy_server_request,
    }
}

#[test]
fn metadata_cleanup_removes_the_span_when_headers_are_merged() {
    let request = ProxyServerRequest {
        headers: IndexMap::from([("helicone_user".to_owned(), "u".to_owned())]),
    };
    let metadata = resolve(&params(Some(request)));
    assert!(!metadata.contains_key("litellm_parent_otel_span"));
    assert_eq!(metadata.len(), 2);
}

#[test]
fn metadata_cleanup_removes_the_span_without_a_proxy_request() {
    let metadata = resolve(&params(None));
    assert_eq!(
        metadata,
        IndexMap::from([("keep".to_owned(), Json::Bool(true))])
    );
}

#[test]
fn metadata_cleanup_leaves_the_callers_metadata_alone() {
    let original = params(None);
    let before = original.metadata.clone();
    let _cleaned = resolve(&original);
    assert_eq!(original.metadata, before);
}
