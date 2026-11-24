use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::model_group_usage::response_headers::{ResponseHeadersRequest, set_response_headers};
use crate::router_core::headers::Response;

use super::support::{
    FailingMetadata, FakeCache, FakeMetadata, deployment, group_info, now, number, object, runtime,
    text,
};

#[test]
fn response_headers() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answered(vec![number(3), Json::Null]);
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(10), None, None, None)),
    };
    let kwargs = object(vec![("metadata", bucket("a"))]);
    let response = Response::Object(object(vec![("content", text("hi"))]));
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response,
            model_group: Some("group"),
            request_kwargs: Some(&kwargs),
            model_list: Some(&rows),
            metadata: &metadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    assert_eq!(hidden(&returned).get("model_id"), Some(&text("a")));
    let headers = headers(&returned);
    assert_eq!(headers.get("x-litellm-model-group"), Some(&text("group")));
    assert_eq!(
        headers.get("x-ratelimit-remaining-tokens"),
        Some(&number(7))
    );
    assert_eq!(headers.get("x-ratelimit-limit-tokens"), Some(&number(10)));
}

#[test]
fn null_response_is_returned_untouched_before_any_read() {
    let cache = FakeCache::failing();
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: Response::Absent,
            model_group: Some("group"),
            request_kwargs: None,
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the untouched response");
    assert_eq!(returned, Response::Absent);
    assert!(cache.batches().is_empty());
}

#[test]
fn null_group_still_stamps_the_group_header() {
    let cache = FakeCache::failing();
    let response = Response::Object(object(vec![("content", text("hi"))]));
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response,
            model_group: None,
            request_kwargs: None,
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    assert_eq!(
        headers(&returned),
        &object(vec![("x-litellm-model-group", Json::Null)])
    );
    assert!(cache.batches().is_empty());
}

#[test]
fn model_id_prefers_litellm_metadata_then_keeps_an_existing_one() {
    let both = object(vec![
        ("litellm_metadata", bucket("litellm")),
        ("metadata", bucket("metadata")),
    ]);
    let existing = Response::Object(object(vec![(
        "_hidden_params",
        Json::Object(object(vec![("model_id", text("existing"))])),
    )]));
    assert_eq!(stamped_model_id(existing, &both), Some(text("existing")));
    let fresh = Response::Object(JsonObject::new());
    assert_eq!(stamped_model_id(fresh, &both), Some(text("litellm")));
}

#[test]
fn model_id_falls_back_to_metadata() {
    let only_metadata = object(vec![("metadata", bucket("metadata"))]);
    let fresh = Response::Object(JsonObject::new());
    assert_eq!(
        stamped_model_id(fresh, &only_metadata),
        Some(text("metadata"))
    );
}

#[test]
fn model_id_skips_a_bucket_without_model_info() {
    let kwargs = object(vec![
        (
            "litellm_metadata",
            Json::Object(object(vec![("other", text("x"))])),
        ),
        ("metadata", bucket("metadata")),
    ]);
    let fresh = Response::Object(JsonObject::new());
    assert_eq!(stamped_model_id(fresh, &kwargs), Some(text("metadata")));
}

#[test]
fn existing_rate_limit_headers_win_over_the_remaining_usage() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answered(vec![number(4), Json::Null]);
    let metadata = FakeMetadata {
        answer: Some(group_info(Some(100), None, None, None)),
    };
    let response = Response::Object(object(vec![(
        "_hidden_params",
        Json::Object(object(vec![(
            "additional_headers",
            Json::Object(object(vec![("x-ratelimit-limit-tokens", number(5))])),
        )])),
    )]));
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response,
            model_group: Some("group"),
            request_kwargs: None,
            model_list: Some(&rows),
            metadata: &metadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    let headers = headers(&returned);
    assert_eq!(headers.get("x-ratelimit-limit-tokens"), Some(&number(5)));
    assert_eq!(
        headers.get("x-ratelimit-remaining-tokens"),
        Some(&number(96))
    );
    assert_eq!(headers.get("x-litellm-model-group"), Some(&text("group")));
}

#[test]
fn a_parsed_response_gets_the_headers_on_its_hidden_params() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answered(vec![number(3), Json::Null]);
    let metadata = FakeMetadata {
        answer: Some(group_info(None, Some(10), None, None)),
    };
    let kwargs = object(vec![("metadata", bucket("a"))]);
    let initial_hidden = object(vec![(
        "additional_headers",
        Json::Object(object(vec![("x-existing", text("kept"))])),
    )]);
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: Response::Attributed(initial_hidden),
            model_group: Some("group"),
            request_kwargs: Some(&kwargs),
            model_list: Some(&rows),
            metadata: &metadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the parsed response");
    let Response::Attributed(hidden) = returned else {
        panic!("the parsed response is no longer attributed");
    };
    assert_eq!(hidden.get("model_id"), Some(&text("a")));
    let headers = object_field(&hidden, "additional_headers");
    assert_eq!(headers.get("x-existing"), Some(&text("kept")));
    assert_eq!(headers.get("x-litellm-model-group"), Some(&text("group")));
    assert_eq!(headers.get("x-ratelimit-limit-requests"), Some(&number(10)));
}

#[test]
fn the_stream_wrapper_comes_back_without_consuming_events() {
    let rows = [deployment("group", "a", "openai/gpt-4o")];
    let cache = FakeCache::answered(vec![number(3), Json::Null]);
    let metadata = FakeMetadata {
        answer: Some(group_info(None, Some(10), None, None)),
    };
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: Response::Stream,
            model_group: Some("group"),
            request_kwargs: None,
            model_list: Some(&rows),
            metadata: &metadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the wrapped stream");
    assert_eq!(returned, Response::Stream);
    assert_eq!(cache.batches().len(), 1);
}

#[test]
fn decision_headers_carry_the_quality_and_complexity_decisions() {
    let kwargs = object(vec![
        (
            "metadata",
            Json::Object(object(vec![(
                "quality_router_decision",
                Json::Object(object(vec![
                    ("routed_model", text("m")),
                    ("quality_tier", text("tier-one")),
                ])),
            )])),
        ),
        (
            "litellm_metadata",
            Json::Object(object(vec![(
                "routing_decision",
                Json::Object(object(vec![
                    ("router_type", text("complexity")),
                    ("tier", text("T2")),
                ])),
            )])),
        ),
    ]);
    let headers = stamped_headers(&kwargs);
    assert_eq!(
        headers.get("x-litellm-quality-router-model"),
        Some(&text("m"))
    );
    assert_eq!(
        headers.get("x-litellm-quality-router-tier"),
        Some(&text("tier-one"))
    );
    assert_eq!(
        headers.get("x-litellm-complexity-router-tier"),
        Some(&text("T2"))
    );
}

fn bucket(id: &str) -> Json {
    Json::Object(object(vec![(
        "model_info",
        Json::Object(object(vec![("id", text(id))])),
    )]))
}

fn body(response: &Response) -> &JsonObject {
    match response {
        Response::Object(dict) => dict,
        Response::Absent | Response::Attributed(_) | Response::Stream | Response::Opaque => {
            panic!("the response is not a dict")
        }
    }
}

fn object_field<'object>(object: &'object JsonObject, field: &str) -> &'object JsonObject {
    match object.get(field) {
        Some(Json::Object(bucket)) => bucket,
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            panic!("the response has no {field} object")
        }
    }
}

fn hidden(response: &Response) -> &JsonObject {
    object_field(body(response), "_hidden_params")
}

fn headers(response: &Response) -> &JsonObject {
    object_field(hidden(response), "additional_headers")
}

// A response with no group to report usage for: the failing collaborators prove the read never happens.
fn stamped_headers(kwargs: &JsonObject) -> JsonObject {
    let cache = FakeCache::failing();
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response: Response::Object(JsonObject::new()),
            model_group: None,
            request_kwargs: Some(kwargs),
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    headers(&returned).clone()
}

fn stamped_model_id(response: Response, kwargs: &JsonObject) -> Option<Json> {
    let cache = FakeCache::failing();
    let returned = runtime()
        .block_on(set_response_headers(ResponseHeadersRequest {
            response,
            model_group: None,
            request_kwargs: Some(kwargs),
            model_list: None,
            metadata: &FailingMetadata,
            cache: &cache,
            now: now(),
        }))
        .expect("the response headers");
    hidden(&returned).get("model_id").cloned()
}
