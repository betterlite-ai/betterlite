#![cfg(test)]

use super::{
    HiddenFacts, NativeRerankErrorPayload, RerankAnswer, RerankCallWire, SessionWire, call_id,
    declared_api_base, hidden_params, model_info_id,
};
use betterlite_core::api::rerank_provider_plan::call::ReturnDocuments;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::ErrorContext;
use betterlite_error::error::details::ResponseDetails;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankResponse};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde_json::Number;

const MODEL: &str = "rerank-english-v3.0";
const PROVIDER: &str = "cohere";
const CALL_ID: &str = "call-1";
const API_BASE: &str = "http://127.0.0.1:8080/v1/rerank";

fn wire(text: &str) -> RerankCallWire {
    serde_json::from_str(text).unwrap()
}

fn map_of(fields: [(&str, Json); 1]) -> IndexMap<String, Json> {
    let mut mapped = IndexMap::new();
    for (name, value) in fields {
        mapped.insert(String::from(name), value);
    }
    mapped
}

fn object(fields: [(&str, Json); 1]) -> Json {
    Json::Object(map_of(fields))
}

#[test]
fn request_arguments_reach_the_call() {
    let parsed = wire(
        r#"{"arguments": {"model": "cohere/rerank-english-v3.0", "query": "q", "documents": ["a", {"text": "b"}], "custom_llm_provider": "cohere", "top_n": 3, "rank_fields": ["text"], "return_documents": false, "max_chunks_per_doc": 10, "max_tokens_per_doc": 20}, "kwargs": {"api_base": "http://example.test"}}"#,
    );
    let call = parsed.into_call();
    assert_eq!(call.model, "cohere/rerank-english-v3.0");
    assert_eq!(call.query, "q");
    assert_eq!(
        call.documents,
        vec![
            RerankDocument::Text(String::from("a")),
            RerankDocument::Object(map_of([("text", Json::Text(String::from("b")))])),
        ]
    );
    assert_eq!(call.custom_llm_provider.as_deref(), Some(PROVIDER));
    assert_eq!(call.top_n, Some(3));
    assert_eq!(call.rank_fields, Some(vec![String::from("text")]));
    assert_eq!(call.return_documents, ReturnDocuments::Value(false));
    assert_eq!(call.max_chunks_per_doc, Some(10));
    assert_eq!(call.max_tokens_per_doc, Some(20));
    assert_eq!(
        call.kwargs.get("api_base"),
        Some(&Json::Text(String::from("http://example.test")))
    );
}

#[test]
fn request_top_n_zero_stays_a_value() {
    let call = wire(r#"{"arguments": {"top_n": 0, "model": "m", "query": "q"}}"#).into_call();
    assert_eq!(call.top_n, Some(0));
}

#[test]
fn request_return_documents_keeps_true_and_false() {
    let true_call = wire(r#"{"arguments": {"return_documents": true}}"#).into_call();
    assert_eq!(true_call.return_documents, ReturnDocuments::Value(true));
    let false_call = wire(r#"{"arguments": {"return_documents": false}}"#).into_call();
    assert_eq!(false_call.return_documents, ReturnDocuments::Value(false));
}

#[test]
fn request_return_documents_null_stays_null() {
    let call = wire(r#"{"arguments": {"return_documents": null}}"#).into_call();
    assert_eq!(call.return_documents, ReturnDocuments::Null);
}

#[test]
fn request_async_arguments_keep_max_tokens_in_kwargs() {
    let args =
        wire(r#"{"kwargs": {"max_tokens_per_doc": 256, "truncate": true}}"#).into_async_args();
    assert_eq!(
        args.kwargs.get("max_tokens_per_doc"),
        Some(&Json::Number(Number::from(256_u32)))
    );
    assert!(args.return_documents.is_none());
    assert_eq!(args.max_chunks_per_doc, None);
}

#[test]
fn request_provider_extras_stay_in_kwargs() {
    let call = wire(r#"{"kwargs": {"truncate": true, "instruction": "pick one", "timeout": 5}}"#)
        .into_call();
    assert_eq!(call.kwargs.get("truncate"), Some(&Json::Bool(true)));
    assert_eq!(
        call.kwargs.get("instruction"),
        Some(&Json::Text(String::from("pick one")))
    );
    assert_eq!(
        call.kwargs.get("timeout"),
        Some(&Json::Number(Number::from(5_u32)))
    );
}

#[test]
fn request_settings_snapshot_is_read() {
    let parsed = wire(
        r#"{"settings": {"api_base": "http://example.test", "api_key": "k", "cohere_key": "c", "drop_params": true, "headers": {"x-one": "1"}, "togetherai_api_key": "t"}}"#,
    );
    let settings = parsed.settings;
    assert_eq!(settings.api_base.as_deref(), Some("http://example.test"));
    assert_eq!(settings.api_key.as_ref().unwrap().expose_secret(), "k");
    assert_eq!(settings.cohere_key.as_ref().unwrap().expose_secret(), "c");
    assert_eq!(settings.drop_params, Some(true));
    assert_eq!(
        settings
            .headers
            .as_ref()
            .and_then(|headers| headers.get("x-one"))
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        settings
            .togetherai_api_key
            .as_ref()
            .unwrap()
            .expose_secret(),
        "t"
    );
}

#[test]
fn request_declared_api_base_reads_both_names() {
    let mut kwargs = IndexMap::new();
    assert_eq!(declared_api_base(&kwargs), None);
    kwargs.insert(String::from("api_base"), Json::Text(String::new()));
    assert_eq!(declared_api_base(&kwargs), Some(String::new()));
    kwargs.insert(String::from("api_base"), Json::Null);
    kwargs.insert(String::from("base_url"), Json::Text(String::new()));
    assert_eq!(declared_api_base(&kwargs), None);
    kwargs.insert(
        String::from("base_url"),
        Json::Text(String::from("http://alias.test")),
    );
    assert_eq!(
        declared_api_base(&kwargs),
        Some(String::from("http://alias.test"))
    );
}

#[test]
fn request_model_id_reads_model_info() {
    let mut kwargs = IndexMap::new();
    kwargs.insert(
        String::from("model_info"),
        object([("id", Json::Text(String::from("deployment-1")))]),
    );
    assert_eq!(
        model_info_id(&kwargs),
        Some(&Json::Text(String::from("deployment-1")))
    );
    kwargs.insert(String::from("model_info"), object([("other", Json::Null)]));
    assert_eq!(model_info_id(&kwargs), None);
    kwargs.insert(String::from("model_info"), Json::Text(String::from("id")));
    assert_eq!(model_info_id(&kwargs), None);
}

#[test]
fn request_call_id_is_kept_or_generated() {
    let mut kwargs = IndexMap::new();
    assert!(!call_id(&kwargs).is_empty());
    kwargs.insert(String::from("litellm_call_id"), Json::Text(String::new()));
    assert!(!call_id(&kwargs).is_empty());
    kwargs.insert(
        String::from("litellm_call_id"),
        Json::Number(Number::from(1_u32)),
    );
    assert!(!call_id(&kwargs).is_empty());
    kwargs.insert(
        String::from("litellm_call_id"),
        Json::Text(String::from(CALL_ID)),
    );
    assert_eq!(call_id(&kwargs), CALL_ID);
}

#[test]
fn request_hidden_params_carry_the_call() {
    let hidden = hidden_params(&HiddenFacts {
        call_id: CALL_ID,
        api_base: Some(API_BASE),
        litellm_model: MODEL,
        model_id: None,
        provider: PROVIDER,
        response_ms: 2.5_f64,
        overhead_ms: None,
    });
    assert_eq!(
        hidden.get("_response_ms"),
        Some(&Json::Number(Number::from_f64(2.5_f64).unwrap()))
    );
    assert_eq!(
        hidden.get("callback_duration_ms"),
        Some(&Json::Number(Number::from_f64(0.0_f64).unwrap()))
    );
    assert_eq!(
        hidden.get("litellm_call_id"),
        Some(&Json::Text(String::from(CALL_ID)))
    );
    assert_eq!(
        hidden.get("api_base"),
        Some(&Json::Text(String::from(API_BASE)))
    );
    assert_eq!(hidden.get("model_id"), Some(&Json::Null));
    assert_eq!(hidden.get("response_cost"), Some(&Json::Null));
}

#[test]
fn request_hidden_params_name_the_model_and_provider() {
    let hidden = hidden_params(&HiddenFacts {
        call_id: CALL_ID,
        api_base: Some(API_BASE),
        litellm_model: MODEL,
        model_id: Some(&Json::Text(String::from("deployment-1"))),
        provider: PROVIDER,
        response_ms: 2.5_f64,
        overhead_ms: None,
    });
    assert_eq!(
        hidden.get("additional_headers"),
        Some(&Json::Object(IndexMap::new()))
    );
    assert_eq!(
        hidden.get("litellm_model_name"),
        Some(&Json::Text(String::from(MODEL)))
    );
    assert_eq!(
        hidden.get("custom_llm_provider"),
        Some(&Json::Text(String::from(PROVIDER)))
    );
    assert_eq!(
        hidden.get("model_id"),
        Some(&Json::Text(String::from("deployment-1")))
    );
    assert!(!hidden.contains_key("litellm_overhead_time_ms"));
}

#[test]
fn request_hidden_params_add_the_overhead_only_when_asked() {
    let hidden = hidden_params(&HiddenFacts {
        call_id: CALL_ID,
        api_base: None,
        litellm_model: MODEL,
        model_id: None,
        provider: PROVIDER,
        response_ms: 4.0_f64,
        overhead_ms: Some(1.25_f64),
    });
    assert_eq!(hidden.get("api_base"), Some(&Json::Null));
    assert_eq!(
        hidden.get("litellm_overhead_time_ms"),
        Some(&Json::Number(Number::from_f64(1.25_f64).unwrap()))
    );
}

#[test]
fn request_answer_flattens_the_response() {
    let answer = RerankAnswer {
        response: RerankResponse::default(),
        provider: String::from(PROVIDER),
        hidden_params: hidden_params(&HiddenFacts {
            call_id: CALL_ID,
            api_base: Some(API_BASE),
            litellm_model: MODEL,
            model_id: None,
            provider: PROVIDER,
            response_ms: 2.5_f64,
            overhead_ms: None,
        }),
        response_ms: 2.5_f64,
    };
    let text = serde_json::to_string(&answer).unwrap();
    let Json::Object(fields) = serde_json::from_str::<Json>(&text).unwrap() else {
        panic!("the answer is not an object");
    };
    for name in [
        "id",
        "results",
        "meta",
        "provider",
        "hidden_params",
        "response_ms",
    ] {
        assert!(fields.contains_key(name), "{name} is missing");
    }
    assert_eq!(
        fields.get("provider"),
        Some(&Json::Text(String::from(PROVIDER)))
    );
}

#[test]
fn request_session_wire_reads_the_environment() {
    let parsed: SessionWire =
        serde_json::from_str(r#"{"environment": {"COHERE_API_KEY": "k"}}"#).unwrap();
    let environment = parsed.environment.unwrap();
    assert_eq!(
        environment.get("COHERE_API_KEY").map(String::as_str),
        Some("k")
    );
}

#[test]
fn error_payload_keeps_the_authentication_class() {
    let class = ErrorClass::Authentication(Box::new(ResponseDetails::<401>::from(ErrorContext {
        message: String::from("AuthenticationError: CohereException - bad key"),
        model: Some(String::from(MODEL)),
        provider: Some(String::from(PROVIDER)),
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    })));
    let payload = NativeRerankErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "AuthenticationError");
    assert_eq!(payload.status_code, Some(401));
    assert_eq!(payload.model.as_deref(), Some(MODEL));
    assert_eq!(payload.llm_provider.as_deref(), Some(PROVIDER));
    assert_eq!(
        payload.message,
        "AuthenticationError: CohereException - bad key"
    );
    assert!(payload.headers.is_empty());
}

#[test]
fn error_payload_serializes_every_key() {
    let class = ErrorClass::Unclassified(Box::new(ErrorContext {
        message: String::from("Mock error"),
        model: None,
        provider: None,
        max_retries: None,
        num_retries: None,
        body_type: None,
        body_param: None,
        body_code: None,
    }));
    let payload = NativeRerankErrorPayload::from_class(&class);
    assert_eq!(payload.kind, "Exception");
    assert_eq!(payload.status_code, None);
    let text = serde_json::to_string(&payload).unwrap();
    let Json::Object(fields) = serde_json::from_str(&text).unwrap() else {
        panic!("the error payload is not an object");
    };
    let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "type",
            "message",
            "status_code",
            "llm_provider",
            "model",
            "body",
            "headers",
            "num_retries",
            "max_retries",
        ]
    );
}
