#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;

use betterlite_models::controls::call_params::DropParams;
use betterlite_models::decisions::DecisionsJson;
use betterlite_models::decisions::ir::{DecisionInput, DecisionRequest};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::params::ParameterError;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::perplexity_decisions::PerplexityDecisionsConfig;
use betterlite_openai::perplexity_embedding::reply::MAX_REPLY_BYTES;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_prompts::arguments::json::{Json, Object};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, Method};
use rstest::rstest;
use secrecy::SecretString;
use url::Url;

use crate::aiohttp_openai_dispatch::{AiohttpOpenAiPrepared, AiohttpOpenAiRequest};
use crate::hosted_platform_dispatch::call::{EmbeddingCall, ResourceCall};
use crate::hosted_platform_dispatch::context::DispatchSettings;
use crate::hosted_platform_dispatch::embedding::dispatch;
use crate::hosted_platform_dispatch::error::DispatchError;
use crate::hosted_platform_dispatch::perplexity;
use crate::hosted_platform_dispatch::perplexity::adapter::Perplexity;
use crate::operation::{EmbeddingOperation, OperationError, OperationRequest, OperationResult};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::{Adapter as _, Provider};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn body(bytes: &[u8]) -> Json {
    serde_json::from_slice(bytes).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn embedding_call() -> EmbeddingCall {
    EmbeddingCall {
        model: String::from("pplx-embed-v1-0.6b"),
        input: EmbeddingInput::Text(String::from("hello")),
        optional_params: object(r#"{"encoding_format": "float"}"#),
        litellm_params: object(r#"{"ssl_verify": false}"#),
        api_key: Some(SecretString::from(String::from("sk-passed"))),
        api_base: Some(String::from("https://mock.example")),
        headers: Some(BTreeMap::from([(
            String::from("X-Caller"),
            String::from("c1"),
        )])),
        timeout: Duration::from_secs(1),
        aembedding: false,
    }
}

fn embedding_operation(text: &str) -> OperationRequest {
    OperationRequest::Embedding(Box::new(EmbeddingOperation {
        model: String::from("pplx-embed-v1-0.6b"),
        request: serde_json::from_str(text).unwrap(),
    }))
}

fn perplexity_deployment(api_key: Option<&str>) -> Deployment {
    Deployment {
        model: String::from("pplx-embed-v1-0.6b"),
        api_base: String::from("https://mock.example"),
        endpoint: Url::parse("https://mock.example").unwrap(),
        api_key: api_key.map(|key| SecretString::from(key.to_owned())),
        authorization: None,
        organization: None,
        timeout: Duration::from_secs(3),
        max_retries: None,
        drop_params: None,
        global_drop_params: false,
        provider_affinity_header: None,
        anthropic: None,
    }
}

fn catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn prepared_embedding() -> PreparedRequest {
    Perplexity
        .prepare(
            &embedding_operation(r#"{"input":"hello"}"#),
            &perplexity_deployment(Some("sk-deployment")),
            &catalog(),
        )
        .unwrap()
}

fn buffered(status: u16, text: &str) -> Upstream {
    Upstream::Buffered(UpstreamResponse {
        status,
        headers: HeaderMap::new(),
        body: text.as_bytes().to_vec(),
    })
}

fn resource_call<'call>(
    model: &'call str,
    optional_params: &'call Object,
    headers: &'call BTreeMap<String, String>,
    api_key: Option<&'call SecretString>,
) -> ResourceCall<'call> {
    ResourceCall {
        model,
        api_base: Some("https://mock.example"),
        api_key,
        headers: Some(headers),
        optional_params,
        litellm_params: optional_params,
    }
}

#[test]
fn reaches_config_embedding_through_the_perplexity_arm() {
    let call = embedding_call();
    let arm = dispatch(ProviderId::Perplexity, &call, DispatchSettings::default())
        .expect("perplexity has an arm of its own")
        .unwrap();
    assert!(arm.litellm_params.is_empty());
    assert_eq!(arm.headers, None);
    assert_eq!(arm.api_base.as_deref(), Some("https://mock.example"));
    assert_eq!(arm.model, call.model);
    assert_eq!(arm.optional_params, call.optional_params);
}

#[test]
fn reaches_config_embedding_through_the_perplexity_adapter() {
    let provider = Provider::from_slug("perplexity").unwrap();
    assert_eq!(provider, Provider::Perplexity(Perplexity));
    assert_eq!(provider.name(), "perplexity");
    let operation = embedding_operation(
        r#"{"model":"pplx","input":"hello","encoding_format":"float","dimensions":8}"#,
    );
    let prepared = provider
        .prepare(
            &operation,
            &perplexity_deployment(Some("sk-deployment")),
            &catalog(),
        )
        .unwrap();
    let PreparedRequest::Embedding(sent) = &prepared else {
        panic!("an embedding request");
    };
    assert_eq!(sent.http.method, Method::POST);
    assert_eq!(sent.http.url.as_str(), "https://mock.example/v1/embeddings");
    let authorization = sent.http.headers.get(AUTHORIZATION).unwrap();
    assert_eq!(authorization.to_str().unwrap(), "Bearer sk-deployment");
    assert!(authorization.is_sensitive());
    let content_type = sent.http.headers.get(CONTENT_TYPE).unwrap();
    assert_eq!(content_type.to_str().unwrap(), "application/json");
    assert_eq!(sent.http.headers.len(), 2);
    assert_eq!(sent.http.deadline, Duration::from_secs(3));
    assert_eq!(sent.client_model.as_deref(), Some("pplx"));
    assert_eq!(
        body(&sent.http.body),
        json(
            r#"{"model":"pplx-embed-v1-0.6b","input":"hello","encoding_format":"float","dimensions":8}"#
        )
    );
}

#[test]
fn reaches_config_embedding_user_without_drop_params_is_refused() {
    let refused = Perplexity
        .prepare(
            &embedding_operation(r#"{"input":"hello","user":"u1"}"#),
            &perplexity_deployment(Some("sk-deployment")),
            &catalog(),
        )
        .unwrap_err();
    let OperationError::Parameters {
        source: ParameterError::Unsupported { model, names },
    } = refused
    else {
        panic!("a set user is refused");
    };
    assert_eq!(model, "pplx-embed-v1-0.6b");
    assert_eq!(names, [String::from("user")]);
}

#[rstest]
#[case::call_drop_params(true)]
#[case::global_drop_params(false)]
fn reaches_config_embedding_user_with_drop_params_is_dropped(#[case] call_flag: bool) {
    let deployment = Deployment {
        drop_params: call_flag.then_some(DropParams::Flag(true)),
        global_drop_params: !call_flag,
        ..perplexity_deployment(Some("sk-deployment"))
    };
    let prepared = Perplexity
        .prepare(
            &embedding_operation(r#"{"input":"hello","user":"u1"}"#),
            &deployment,
            &catalog(),
        )
        .unwrap();
    let PreparedRequest::Embedding(sent) = &prepared else {
        panic!("an embedding request");
    };
    assert_eq!(
        body(&sent.http.body),
        json(r#"{"model":"pplx-embed-v1-0.6b","input":"hello"}"#)
    );
}

#[test]
fn reaches_config_embedding_extra_fields_reach_the_body() {
    let prepared = Perplexity
        .prepare(
            &embedding_operation(
                r#"{"input":"hello","encoding_format":"float","dimensions":8,"foo":"bar","dropped":null,"extra_body":{"baz":1,"model":"other","kept":null}}"#,
            ),
            &perplexity_deployment(Some("sk-deployment")),
            &catalog(),
        )
        .unwrap();
    let PreparedRequest::Embedding(sent) = &prepared else {
        panic!("an embedding request");
    };
    assert_eq!(
        body(&sent.http.body),
        json(
            r#"{"model":"pplx-embed-v1-0.6b","input":"hello","encoding_format":"float","dimensions":8,"foo":"bar","baz":1,"kept":null}"#
        )
    );
}

#[test]
fn reaches_config_embedding_extra_body_must_be_an_object() {
    let refused = Perplexity
        .prepare(
            &embedding_operation(r#"{"input":"hello","extra_body":"nope"}"#),
            &perplexity_deployment(Some("sk-deployment")),
            &catalog(),
        )
        .unwrap_err();
    let OperationError::Parameters {
        source: ParameterError::NotAnObject { name },
    } = refused
    else {
        panic!("a non-object extra_body is refused");
    };
    assert_eq!(name, "extra_body");
}

#[test]
fn reaches_config_embedding_mock_controls_stay_out_of_the_body() {
    let prepared = Perplexity
        .prepare(
            &embedding_operation(
                r#"{"input":"hello","mock_response":[0.5,0.25],"mock_tool_calls":[{"id":"call-1"}]}"#,
            ),
            &perplexity_deployment(Some("sk-deployment")),
            &catalog(),
        )
        .unwrap();
    let PreparedRequest::Embedding(sent) = &prepared else {
        panic!("an embedding request");
    };
    assert_eq!(
        body(&sent.http.body),
        json(r#"{"model":"pplx-embed-v1-0.6b","input":"hello"}"#)
    );
}

#[test]
fn reaches_config_embedding_without_a_key_sends_bearer_none() {
    let prepared = Perplexity
        .prepare(
            &embedding_operation(r#"{"input":["a","b"]}"#),
            &perplexity_deployment(None),
            &catalog(),
        )
        .unwrap();
    let PreparedRequest::Embedding(sent) = &prepared else {
        panic!("an embedding request");
    };
    assert_eq!(
        sent.http
            .headers
            .get(AUTHORIZATION)
            .unwrap()
            .to_str()
            .unwrap(),
        "Bearer None"
    );
    assert_eq!(
        body(&sent.http.body),
        json(r#"{"model":"pplx-embed-v1-0.6b","input":["a","b"]}"#)
    );
}

#[test]
fn reaches_config_embedding_and_refuses_other_operations() {
    let refused = Perplexity
        .prepare(
            &OperationRequest::AiohttpOpenAi(AiohttpOpenAiRequest),
            &perplexity_deployment(None),
            &catalog(),
        )
        .unwrap_err();
    let OperationError::UnsupportedOperation { provider, .. } = refused else {
        panic!("only an embedding runs through perplexity");
    };
    assert_eq!(provider, "perplexity");
    let other = PreparedRequest::AiohttpOpenAi(AiohttpOpenAiPrepared);
    let Err(decoded) = Perplexity.decode(&other, buffered(200, "{}")) else {
        panic!("only an embedding reply is decoded");
    };
    assert!(matches!(
        decoded,
        OperationError::UnsupportedOperation { .. }
    ));
}

#[test]
fn reaches_config_embedding_reply_decodes_int8_into_floats() {
    let reply = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":"AH+B"}],"usage":{"total_tokens":2}}"#;
    let OperationResult::Embedding(response) = Perplexity
        .decode(&prepared_embedding(), buffered(200, reply))
        .unwrap()
    else {
        panic!("an embedding response");
    };
    assert_eq!(response.model, "pplx-embed-v1-0.6b");
    assert_eq!(response.object, "list");
    assert_eq!(response.usage.prompt_tokens, 2);
    assert_eq!(response.usage.total_tokens, 2);
    assert_eq!(
        serde_json::to_string(&response.data[0].embedding).unwrap(),
        "[0.0,1.0,-1.0]"
    );
}

#[test]
fn reaches_config_embedding_reply_names_the_client_model() {
    let prepared = Perplexity
        .prepare(
            &embedding_operation(r#"{"model":"pplx-embed","input":"hello"}"#),
            &perplexity_deployment(Some("sk-deployment")),
            &catalog(),
        )
        .unwrap();
    let reply = r#"{"object":"list","data":[{"object":"embedding","index":0,"embedding":"AH+B"}],"model":"pplx-embed-v1-0.6b","usage":{"total_tokens":2}}"#;
    let OperationResult::Embedding(response) =
        Perplexity.decode(&prepared, buffered(200, reply)).unwrap()
    else {
        panic!("an embedding response");
    };
    assert_eq!(response.model, "pplx-embed");
}

#[rstest]
#[case::provider_status(429, r#"{"error":"slow down"}"#)]
#[case::not_json(200, "upstream broke")]
fn reaches_config_embedding_reply_keeps_the_provider_failure(
    #[case] status: u16,
    #[case] text: &str,
) {
    let Err(OperationError::Provider { response }) =
        Perplexity.decode(&prepared_embedding(), buffered(status, text))
    else {
        panic!("the provider's own failure");
    };
    assert_eq!(response.status, status);
    assert_eq!(response.body, text.as_bytes());
}

#[rstest]
#[case::bad_base64(r#"{"data":[{"object":"embedding","index":0,"embedding":"A"}]}"#)]
#[case::wrong_shape(r#"{"data":"nope"}"#)]
#[case::missing_record_fields(r#"{"data":[{"embedding":[0.5]}]}"#)]
fn reaches_config_embedding_reply_of_a_bad_shape_is_a_decode_error(#[case] text: &str) {
    let Err(failed) = Perplexity.decode(&prepared_embedding(), buffered(200, text)) else {
        panic!("a bad reply is a decode error");
    };
    assert!(matches!(
        failed,
        OperationError::Decode {
            operation: "embedding",
            ..
        }
    ));
}

#[test]
fn reaches_config_embedding_reply_over_the_limit_is_too_large() {
    let text = " ".repeat(MAX_REPLY_BYTES.checked_add(1).unwrap());
    let Err(failed) = Perplexity.decode(&prepared_embedding(), buffered(200, &text)) else {
        panic!("an oversized reply is refused");
    };
    assert!(matches!(failed, OperationError::TooLarge { limit } if limit == MAX_REPLY_BYTES));
}

#[test]
fn reaches_config_responses_with_a_preset_body() {
    let params = object(r#"{"max_output_tokens": 20}"#);
    let headers = BTreeMap::from([(String::from("X-Caller"), String::from("c1"))]);
    let call = resource_call("preset/pro-search", &params, &headers, None);
    let input: ResponsesInput =
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap();
    let request = perplexity::responses(
        &call,
        &input,
        &environment(&[("PERPLEXITY_API_KEY", "sk-env")]),
    )
    .unwrap();
    assert_eq!(request.url, "https://mock.example/v1/responses");
    assert_eq!(
        request.headers,
        BTreeMap::from([
            (String::from("Authorization"), String::from("Bearer sk-env")),
            (String::from("X-Caller"), String::from("c1")),
        ])
    );
    assert_eq!(
        body(&request.body),
        json(
            r#"{"preset":"pro-search","input":[{"role":"user","content":"hi","type":"message"}],"max_output_tokens":20}"#
        )
    );
}

#[test]
fn reaches_config_responses_and_reports_a_bad_tools_value() {
    let params = object(r#"{"tools": "not a list"}"#);
    let headers = BTreeMap::new();
    let call = resource_call("sonar", &params, &headers, None);
    let input: ResponsesInput = serde_json::from_str(r#""hi""#).unwrap();
    let outcome = perplexity::responses(&call, &input, &environment(&[]));
    assert!(matches!(
        outcome,
        Err(DispatchError::PerplexityResponses(_))
    ));
}

#[test]
fn reaches_config_search_with_the_four_options() {
    let params = object(r#"{"max_results": 3, "search_recency_filter": "week"}"#);
    let headers = BTreeMap::new();
    let key = SecretString::from(String::from("sk-call"));
    let call = resource_call("search", &params, &headers, Some(&key));
    let query = SearchQuery::Single(String::from("rust"));
    let request = perplexity::search(&call, &query, &environment(&[])).unwrap();
    assert_eq!(request.url, "https://mock.example/search");
    assert_eq!(
        request.headers,
        BTreeMap::from([
            (
                String::from("Authorization"),
                String::from("Bearer sk-call")
            ),
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
        ])
    );
    assert_eq!(
        body(&request.body),
        json(r#"{"query":"rust","max_results":3}"#)
    );
}

#[test]
fn reaches_config_search_and_names_the_missing_key() {
    let params = Object::new();
    let headers = BTreeMap::new();
    let call = resource_call("search", &params, &headers, None);
    let query = SearchQuery::Single(String::from("rust"));
    let outcome = perplexity::search(
        &call,
        &query,
        &environment(&[("PERPLEXITY_API_KEY", "sk-other")]),
    );
    let Err(DispatchError::PerplexitySearch(error)) = outcome else {
        panic!("the search key is required");
    };
    assert_eq!(
        error.to_string(),
        "PERPLEXITYAI_API_KEY is not set. Set `PERPLEXITYAI_API_KEY` environment variable."
    );
}

#[test]
fn reaches_config_decisions_with_its_path_and_variables() {
    let params = Object::new();
    let headers = BTreeMap::from([(String::from("X-Caller"), String::from("c1"))]);
    let mut call = resource_call("pplx-decider", &params, &headers, None);
    call.api_base = Some("https://mock.example/v1/");
    let request = DecisionRequest {
        input: DecisionInput::State(DecisionsJson::Text(String::from("ready"))),
        questions: PerplexityDecisionsConfig.health_check_questions(),
        safety_identifier: None,
    };
    let built = perplexity::decisions(
        &call,
        &request,
        &environment(&[
            ("PERPLEXITYAI_API_KEY", ""),
            ("PERPLEXITY_API_KEY", "sk-second"),
        ]),
    )
    .unwrap();
    assert_eq!(built.url, "https://mock.example/v1/decisions");
    assert_eq!(
        built.headers,
        BTreeMap::from([
            (
                String::from("Authorization"),
                String::from("Bearer sk-second")
            ),
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
            (String::from("X-Caller"), String::from("c1")),
        ])
    );
    assert!(!built.body.is_empty());
}

#[test]
fn reaches_config_decisions_from_the_base_variable_and_the_default() {
    let params = Object::new();
    let headers = BTreeMap::new();
    let mut call = resource_call("pplx-decider", &params, &headers, None);
    let request = DecisionRequest {
        input: DecisionInput::State(DecisionsJson::Text(String::from("ready"))),
        questions: PerplexityDecisionsConfig.health_check_questions(),
        safety_identifier: None,
    };
    call.api_base = Some("");
    let from_variable = perplexity::decisions(
        &call,
        &request,
        &environment(&[("PERPLEXITY_API_BASE", "https://env.example")]),
    )
    .unwrap();
    assert_eq!(from_variable.url, "https://env.example/v1/decisions");
    assert!(!from_variable.headers.contains_key("Authorization"));
    call.api_base = None;
    let defaulted = perplexity::decisions(&call, &request, &environment(&[])).unwrap();
    assert_eq!(defaulted.url, "https://api.perplexity.ai/v1/decisions");
}

#[test]
fn reaches_config_decisions_with_the_call_key_over_the_variables() {
    let params = Object::new();
    let headers = BTreeMap::new();
    let request = DecisionRequest {
        input: DecisionInput::State(DecisionsJson::Text(String::from("ready"))),
        questions: PerplexityDecisionsConfig.health_check_questions(),
        safety_identifier: None,
    };
    let variables = environment(&[("PERPLEXITYAI_API_KEY", "sk-env")]);
    let key = SecretString::from(String::from("sk-call"));
    let keyed = resource_call("pplx-decider", &params, &headers, Some(&key));
    let from_call = perplexity::decisions(&keyed, &request, &variables).unwrap();
    assert_eq!(
        from_call.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-call")
    );
    let empty = SecretString::from(String::new());
    let unkeyed = resource_call("pplx-decider", &params, &headers, Some(&empty));
    let from_variable = perplexity::decisions(&unkeyed, &request, &variables).unwrap();
    assert_eq!(
        from_variable
            .headers
            .get("Authorization")
            .map(String::as_str),
        Some("Bearer sk-env")
    );
}
