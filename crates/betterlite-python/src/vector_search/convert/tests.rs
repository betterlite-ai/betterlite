#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_core::api::vector_search_execution::config::SearchQuery;
use betterlite_core::api::vector_search_execution::error::SearchError;
use betterlite_core::api::vector_store_http_create_read::error::CreateReadError;
use betterlite_core::api::vector_store_http_write::error::{
    BaseLlmException, HeaderMap as WireHeaders, WriteError,
};
use betterlite_core::api::vector_store_search::error::VectorStoreSearchError;
use betterlite_core::api::vector_store_search::service::Entry;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use indexmap::IndexMap;

use super::{
    ConvertError, CreateWire, NativeErrorPayload, NativeRaise, SearchWire, create_call, parse,
    raise_payload, search_call,
};

const CREATE_TEXT: &str = r#"{"optional":{"name":"notes","file_ids":["file_a"],"expires_after":{"anchor":"last_active_at","days":7},"chunking_strategy":{"type":"auto"},"metadata":{"tag":"x"}},"kwargs":{"api_key":"sk-test","api_base":"http://127.0.0.1:9/v1","ssl_verify":true},"extra_headers":{"x-test":"1"},"extra_body":{"custom":true},"timeout":30.5,"custom_llm_provider":"openai","entry":"sync"}"#;
const SEARCH_TEXT: &str = r#"{"vector_store_id":"vs_1","query":["a","b"],"optional":{"filters":{"type":"eq","key":"tag","value":"x"},"max_num_results":5,"ranking_options":{"ranker":"auto"},"rewrite_query":false},"kwargs":{"api_key":"sk-test"},"extra_headers":{"x-test":"1"},"extra_body":{"custom":true},"timeout":12.0,"custom_llm_provider":"openai","entry":"sync"}"#;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the fixture is valid JSON")
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds")
}

fn exception(status_code: u16) -> BaseLlmException {
    BaseLlmException {
        status_code,
        message: String::from("missing"),
        headers: WireHeaders::new(),
        status_code_is_synthesized: false,
    }
}

#[test]
fn request_create_call_keeps_every_field() {
    let wire: CreateWire = parse(CREATE_TEXT, &[]).expect("the create wire parses");
    let call = create_call(&wire);
    assert_eq!(call.optional.len(), 5);
    assert_eq!(call.optional.get("name"), Some(&json(r#""notes""#)));
    assert_eq!(call.optional.get("file_ids"), Some(&json(r#"["file_a"]"#)));
    assert_eq!(
        call.optional.get("expires_after"),
        Some(&json(r#"{"anchor":"last_active_at","days":7}"#))
    );
    assert_eq!(call.optional.get("metadata"), Some(&json(r#"{"tag":"x"}"#)));
    assert_eq!(call.kwargs.get("api_key"), Some(&json(r#""sk-test""#)));
    assert_eq!(
        call.kwargs.get("api_base"),
        Some(&json(r#""http://127.0.0.1:9/v1""#))
    );
    assert_eq!(call.kwargs.get("ssl_verify"), Some(&json("true")));
    assert_eq!(
        call.extra_headers,
        Some(&BTreeMap::from([(
            String::from("x-test"),
            String::from("1")
        )]))
    );
    assert_eq!(
        call.extra_body,
        Some(&IndexMap::from([(String::from("custom"), json("true"))]))
    );
    assert_eq!(call.timeout, Some(30.5_f64));
    assert_eq!(call.custom_llm_provider, Some("openai"));
    assert_eq!(call.entry, Entry::Sync);
}

#[test]
fn request_create_call_defaults_the_absent_fields() {
    let wire: CreateWire = parse("{}", &[]).expect("an empty create wire parses");
    let call = create_call(&wire);
    assert_eq!(call.optional.len(), 0);
    assert_eq!(call.kwargs.len(), 0);
    assert_eq!(call.extra_headers, None);
    assert_eq!(call.extra_body, None);
    assert_eq!(call.timeout, None);
    assert_eq!(call.custom_llm_provider, None);
    assert_eq!(call.entry, Entry::Sync);
}

#[test]
fn request_create_call_takes_the_async_entry() {
    let wire: CreateWire = parse(r#"{"entry":"async"}"#, &[]).expect("the async entry parses");
    assert_eq!(create_call(&wire).entry, Entry::Asynchronous);
}

#[test]
fn request_search_call_keeps_every_field() {
    let wire: SearchWire = parse(SEARCH_TEXT, &[]).expect("the search wire parses");
    let call = search_call(&wire);
    assert_eq!(call.vector_store_id, "vs_1");
    assert_eq!(
        call.query,
        &SearchQuery::Sequence(vec![String::from("a"), String::from("b")])
    );
    assert_eq!(call.optional.get("max_num_results"), Some(&json("5")));
    assert_eq!(
        call.optional.get("filters"),
        Some(&json(r#"{"type":"eq","key":"tag","value":"x"}"#))
    );
    assert_eq!(
        call.optional.get("ranking_options"),
        Some(&json(r#"{"ranker":"auto"}"#))
    );
    assert_eq!(call.optional.get("rewrite_query"), Some(&json("false")));
    assert_eq!(call.kwargs.get("api_key"), Some(&json(r#""sk-test""#)));
    assert_eq!(call.timeout, Some(12.0_f64));
    assert_eq!(call.custom_llm_provider, Some("openai"));
    assert_eq!(call.entry, Entry::Sync);
    assert!(call.executor.is_none());
}

#[test]
fn request_search_call_takes_a_text_query() {
    let wire: SearchWire =
        parse(r#"{"vector_store_id":"vs","query":"needle"}"#, &[]).expect("the text query parses");
    let call = search_call(&wire);
    assert_eq!(call.query, &SearchQuery::Text(String::from("needle")));
    assert_eq!(call.optional.len(), 0);
    assert_eq!(call.extra_headers, None);
    assert_eq!(call.extra_body, None);
    assert_eq!(call.timeout, None);
    assert_eq!(call.custom_llm_provider, None);
    assert_eq!(call.entry, Entry::Sync);
}

#[test]
fn request_wire_rejects_an_unknown_key() {
    let refused = parse::<SearchWire>(r#"{"vector_store_id":"vs","query":"q","router":null}"#, &[])
        .expect_err("a key the call does not carry is refused");
    assert!(matches!(refused, ConvertError::Json { .. }));
}

#[test]
fn request_wire_rejects_attached_files() {
    let refused =
        parse::<CreateWire>("{}", &[vec![1, 2, 3]]).expect_err("a vector store call takes no file");
    assert!(matches!(refused, ConvertError::Files { count: 1 }));
}

#[test]
fn request_search_wire_requires_its_two_arguments() {
    assert!(matches!(
        parse::<SearchWire>("{}", &[]),
        Err(ConvertError::Json { .. })
    ));
    assert!(matches!(
        parse::<SearchWire>(r#"{"vector_store_id":"vs"}"#, &[]),
        Err(ConvertError::Json { .. })
    ));
}

#[test]
fn request_create_text_that_is_not_json_is_an_error() {
    assert!(matches!(
        parse::<CreateWire>("not json", &[]),
        Err(ConvertError::Json { .. })
    ));
}

#[test]
fn error_search_failure_is_a_status_class() {
    let error = VectorStoreSearchError::Search {
        provider: String::from("openai"),
        source: SearchError::Provider(exception(404)),
    };
    assert_eq!(
        raise_payload(&error, &redactor()),
        NativeRaise::Exception(NativeErrorPayload {
            kind: String::from("NotFoundError"),
            message: String::from("missing"),
            status_code: 404,
            llm_provider: String::from("openai"),
            model: String::new(),
        })
    );
}

#[test]
fn error_create_failure_keeps_the_provider_and_status() {
    let error = VectorStoreSearchError::Create {
        provider: String::from("openai"),
        source: CreateReadError::Write(WriteError::Provider(exception(401))),
    };
    assert_eq!(
        raise_payload(&error, &redactor()),
        NativeRaise::Exception(NativeErrorPayload {
            kind: String::from("AuthenticationError"),
            message: String::from("missing"),
            status_code: 401,
            llm_provider: String::from("openai"),
            model: String::new(),
        })
    );
}

#[test]
fn error_invalid_executor_is_a_bare_type_error() {
    assert_eq!(
        raise_payload(
            &VectorStoreSearchError::InvalidEmbeddingExecutor,
            &redactor()
        ),
        NativeRaise::TypeError(String::from(
            "Invalid direct vector store embedding executor"
        ))
    );
}

#[test]
fn error_unsupported_provider_is_an_api_connection_error() {
    let error = VectorStoreSearchError::Unsupported {
        operation: "search",
        provider: String::from("anthropic"),
    };
    assert_eq!(
        raise_payload(&error, &redactor()),
        NativeRaise::Exception(NativeErrorPayload {
            kind: String::from("APIConnectionError"),
            message: String::from("Vector store search is not supported for anthropic"),
            status_code: 500,
            llm_provider: String::from("anthropic"),
            model: String::new(),
        })
    );
}
