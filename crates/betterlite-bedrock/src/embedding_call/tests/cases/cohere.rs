use std::time::Duration;

use betterlite_models::decisions::Json as Decisions;
use betterlite_openai::cohere_embedding::usage::BilledUnits;
use betterlite_prompts::arguments::json::Json;

use super::super::fixture::{Probe, Reply, object, run};
use super::super::support::{argument_fields, legacy, logged_response, phases};
use crate::bedrock_embedding_io::transport::{ClientKind, Timeout};
use crate::embedding_call::error::EmbeddingCallError;

const COHERE_MODEL: &str = "cohere.embed-english-v3";
const REPLY: &str =
    "{\"embeddings\":[[0.1,0.2],[0.3,0.4]],\"meta\":{\"billed_units\":{\"input_tokens\":7}}}";

pub(super) fn cohere_batched_input() {
    one_body_holds_every_input();
    options_are_filtered_to_declared_fields();
    the_caller_timeout_does_not_reach_the_sync_handler();
    async_logging_carries_the_prepared_request();
    usage_falls_back_to_the_encoding();
    headers_keep_their_precedence();
    a_bad_status_is_a_typed_failure();
    only_a_matching_client_replaces_the_handler();
}

fn one_body_holds_every_input() {
    let outcome = run(&Probe {
        model: String::from(COHERE_MODEL),
        input: vec![String::from("one"), String::from("two")],
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert_eq!(outcome.cohere_requests.len(), 1);
    assert!(outcome.factory_requests.is_empty());
    assert_eq!(
        outcome.posted().url,
        "https://bedrock-runtime.us-east-1.amazonaws.com/model/cohere.embed-english-v3/invoke"
    );
    assert_eq!(
        outcome.body(),
        object("{\"input_type\":\"search_document\",\"texts\":[\"one\",\"two\"]}")
    );
    let reply = legacy(outcome.response());
    assert_eq!(reply.object, "list");
    assert_eq!(reply.data.len(), 2);
    assert_eq!(reply.data[0].index, 0);
    assert_eq!(reply.data[1].index, 1);
    assert_eq!(numbers(&reply.data[0].embedding), [0.1, 0.2]);
    assert_eq!(numbers(&reply.data[1].embedding), [0.3, 0.4]);
    assert_eq!(reply.model, COHERE_MODEL);
    assert_eq!(reply.usage.prompt_tokens, 7);
    assert_eq!(reply.usage.completion_tokens, 0);
    assert_eq!(reply.usage.total_tokens, 7);
    assert_eq!(
        reply.usage.prompt_tokens_details,
        Some(BilledUnits {
            text_tokens: Some(7),
            image_tokens: None,
        })
    );
    assert!(reply.data[0].kind.is_none());
}

fn options_are_filtered_to_declared_fields() {
    let mut options = credentials();
    options.push(("input_type", "\"search_query\""));
    options.push(("truncate", "\"NONE\""));
    options.push(("dimensions", "128"));
    let outcome = run(&Probe {
        model: String::from(COHERE_MODEL),
        optional_params: options,
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert_eq!(
        outcome.body(),
        object("{\"input_type\":\"search_query\",\"texts\":[\"hello\"],\"truncate\":\"NONE\"}")
    );
    assert!(outcome.remaining.contains_key("dimensions"));
}

fn the_caller_timeout_does_not_reach_the_sync_handler() {
    let outcome = run(&Probe {
        model: String::from(COHERE_MODEL),
        timeout: Some(Timeout::Seconds(2.5)),
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert_eq!(outcome.posted().deadline, Some(Duration::from_secs(600)));
    assert!(outcome.calls.is_empty());
    assert_eq!(
        phases(&outcome.snapshots),
        ["pre_api_call", "post_api_call"]
    );
    let args = argument_fields(&outcome.snapshots[0]);
    assert!(!args.contains_key("api_base"));
    assert!(!args.contains_key("headers"));
}

fn async_logging_carries_the_prepared_request() {
    let outcome = run(&Probe {
        model: String::from(COHERE_MODEL),
        timeout: Some(Timeout::Seconds(2.5)),
        aembedding: true,
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert_eq!(
        outcome.posted().deadline,
        Some(Duration::from_secs_f64(2.5))
    );
    assert!(outcome.calls.is_empty());
    assert_eq!(
        phases(&outcome.snapshots),
        ["pre_api_call", "post_api_call"]
    );
    let args = argument_fields(&outcome.snapshots[0]);
    assert_eq!(
        args.get("api_base"),
        Some(&Decisions::Text(outcome.posted().url.clone()))
    );
    let Some(Decisions::Object(headers)) = args.get("headers") else {
        panic!("the async pre_call logs the prepared headers");
    };
    assert!(headers.contains_key("Authorization"));
    assert!(headers.contains_key("X-Amz-Date"));
    assert_eq!(
        logged_response(&outcome),
        parsed(
            "{\"embeddings\":[[0.1,0.2],[0.3,0.4]],\"meta\":{\"billed_units\":{\"input_tokens\":7}}}"
        )
    );
}

fn usage_falls_back_to_the_encoding() {
    let outcome = run(&Probe {
        model: String::from(COHERE_MODEL),
        input: vec![String::from("one"), String::from("two")],
        replies: vec![Reply::ok("{\"embeddings\":[[0.1],[0.2]]}")],
        ..Probe::default()
    });
    let reply = legacy(outcome.response());
    assert_eq!(reply.usage.prompt_tokens, 6);
    assert_eq!(reply.usage.total_tokens, 6);
    assert!(reply.usage.prompt_tokens_details.is_none());
}

fn headers_keep_their_precedence() {
    let overridden = run(&Probe {
        model: String::from(COHERE_MODEL),
        extra_headers: Some(vec![
            (String::from("Content-Type"), String::from("text/plain")),
            (String::from("X-Tenant"), String::from("abc")),
        ]),
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert_eq!(overridden.headers()["content-type"], "text/plain");
    assert_eq!(overridden.headers()["x-tenant"], "abc");
    assert_eq!(
        overridden.headers()["request-source"],
        "unspecified:litellm"
    );
    assert_eq!(overridden.headers()["accept"], "application/json");
    let basic = run(&Probe {
        model: String::from(COHERE_MODEL),
        extra_headers: Some(vec![(
            String::from("Authorization"),
            String::from("Basic abc"),
        )]),
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert_eq!(basic.headers()["authorization"], "Basic abc");
    let aws4 = run(&Probe {
        model: String::from(COHERE_MODEL),
        extra_headers: Some(vec![(
            String::from("Authorization"),
            String::from("AWS4-HMAC-SHA256 forged"),
        )]),
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert!(aws4.headers()["authorization"].starts_with("AWS4-HMAC-SHA256 Credential="));
}

fn a_bad_status_is_a_typed_failure() {
    let outcome = run(&Probe {
        model: String::from(COHERE_MODEL),
        replies: vec![Reply::status(400, &[], "bad request")],
        ..Probe::default()
    });
    assert!(matches!(outcome.error(), EmbeddingCallError::Cohere(_)));
    assert_eq!(outcome.error().status(), Some(400));
    assert_eq!(outcome.cohere_requests.len(), 1);
    assert!(outcome.original_response().is_none());
    let asynchronous = run(&Probe {
        model: String::from(COHERE_MODEL),
        aembedding: true,
        replies: vec![Reply::status(400, &[], "bad request")],
        ..Probe::default()
    });
    assert_eq!(asynchronous.error().status(), Some(400));
    assert_eq!(
        logged_response(&asynchronous),
        Decisions::Text(String::from("bad request"))
    );
}

fn only_a_matching_client_replaces_the_handler() {
    let supplied = run(&Probe {
        model: String::from(COHERE_MODEL),
        supplied: Some(ClientKind::Sync),
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert_eq!(supplied.supplied_requests.len(), 1);
    assert!(supplied.cohere_requests.is_empty());
    let other_kind = run(&Probe {
        model: String::from(COHERE_MODEL),
        supplied: Some(ClientKind::Async),
        replies: vec![Reply::ok(REPLY)],
        ..Probe::default()
    });
    assert!(other_kind.supplied_requests.is_empty());
    assert_eq!(other_kind.cohere_requests.len(), 1);
}

fn credentials() -> Vec<(&'static str, &'static str)> {
    vec![
        ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
        (
            "aws_secret_access_key",
            "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
        ),
        ("aws_region_name", "\"us-east-1\""),
    ]
}

fn numbers(value: &Json) -> Vec<f64> {
    let Json::List(items) = value else {
        panic!("the legacy record holds a number list");
    };
    items
        .iter()
        .map(|item| match item {
            Json::Number(number) => number.as_f64().expect("the vector holds finite numbers"),
            _ => panic!("the legacy record holds numbers"),
        })
        .collect()
}

fn parsed(text: &str) -> Decisions {
    let value: Json = serde_json::from_str(text).expect("the expected reply parses");
    crate::embedding_call::decisions_value(&value).expect("the expected reply converts")
}
