#![cfg(test)]

use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankResponse,
};
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use thiserror::Error;

use super::{
    BaseLlmException, BaseRerankConfig, EnvironmentRequest, ExactDecimal, HeaderMap, Headers,
    MapCohereRerankParams, OptionalRerankParams, Params, RerankLogging, RerankModelInfo,
    RerankProviderResponse, ResponseTransform, ReturnDocuments, get_optional_rerank_params,
};

#[derive(Debug, Error)]
enum FixtureError {
    #[error("the fixture mapper refused the parameters")]
    Refused,
}

#[derive(Default)]
struct Fixture {
    calls: Mutex<Vec<Json>>,
    refuse: bool,
}

fn parsed(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn document(source: &str) -> RerankDocument {
    serde_json::from_str(source).unwrap()
}

fn params(source: &str) -> Params {
    serde_json::from_str(source).unwrap()
}

fn model_info(source: &str) -> RerankModelInfo {
    serde_json::from_str(source).unwrap()
}

fn billed_units(source: &str) -> RerankBilledUnits {
    serde_json::from_str(source).unwrap()
}

fn decimal(source: &str) -> Decimal {
    source.parse().unwrap()
}

fn field<'json>(value: &'json Json, name: &str) -> Option<&'json Json> {
    match value {
        Json::Object(fields) => fields.get(name),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn object_of(params: &Params) -> Json {
    Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect::<IndexMap<_, _>>(),
    )
}

fn record(request: &MapCohereRerankParams<'_>) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(
        "non_default_params".to_owned(),
        object_of(request.non_default_params),
    );
    fields.insert("model".to_owned(), Json::Text(request.model.to_owned()));
    fields.insert("drop_params".to_owned(), Json::Bool(request.drop_params));
    fields.insert("query".to_owned(), Json::Text(request.query.to_owned()));
    fields.insert(
        "documents".to_owned(),
        Json::List(
            request
                .documents
                .iter()
                .map(RerankDocument::as_json)
                .collect(),
        ),
    );
    insert_optionals(&mut fields, request);
    Json::Object(fields)
}

fn insert_optionals(fields: &mut IndexMap<String, Json>, request: &MapCohereRerankParams<'_>) {
    fields.insert(
        "custom_llm_provider".to_owned(),
        request
            .custom_llm_provider
            .map_or(Json::Null, |provider| Json::Text(provider.to_owned())),
    );
    fields.insert(
        "top_n".to_owned(),
        request.top_n.map_or(Json::Null, |top_n| {
            Json::Number(serde_json::Number::from(top_n))
        }),
    );
    fields.insert(
        "rank_fields".to_owned(),
        request.rank_fields.map_or(Json::Null, |names| {
            Json::List(names.iter().map(|name| Json::Text(name.clone())).collect())
        }),
    );
    fields.insert(
        "return_documents".to_owned(),
        request.return_documents.map_or(Json::Null, Json::Bool),
    );
    fields.insert(
        "max_chunks_per_doc".to_owned(),
        request.max_chunks_per_doc.map_or(Json::Null, |limit| {
            Json::Number(serde_json::Number::from(limit))
        }),
    );
    fields.insert(
        "max_tokens_per_doc".to_owned(),
        request.max_tokens_per_doc.map_or(Json::Null, |limit| {
            Json::Number(serde_json::Number::from(limit))
        }),
    );
    fields.insert(
        "instruction".to_owned(),
        request
            .instruction
            .map_or(Json::Null, |text| Json::Text(text.to_owned())),
    );
}

impl Fixture {
    fn recorded(&self) -> Vec<Json> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BaseRerankConfig for Fixture {
    type Error = FixtureError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        Ok(request.headers.clone())
    }

    fn supported_rerank_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        if self.refuse {
            return Err(FixtureError::Refused);
        }
        let recorded = record(&request);
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(recorded);
        Ok(Params::from([("mapped".to_owned(), Json::Bool(true))]))
    }
}

fn projection_request<'request>(
    non_default_params: Option<&'request mut Params>,
    documents: &'request [RerankDocument],
    rank_fields: Option<&'request [String]>,
    return_documents: ReturnDocuments,
) -> OptionalRerankParams<'request> {
    OptionalRerankParams {
        non_default_params,
        model: "rerank-model",
        drop_params: false,
        query: "q",
        documents,
        custom_llm_provider: Some("cohere"),
        top_n: Some(0_i64),
        rank_fields,
        return_documents,
        max_chunks_per_doc: Some(0_i64),
        max_tokens_per_doc: Some(3_i64),
        instruction: Some(""),
    }
}

fn project_fixture(fixture: &Fixture, documents: &[RerankDocument]) -> Params {
    let mut original = params(r#"{"query":"old","top_n":9,"extension":"yes"}"#);
    get_optional_rerank_params(
        fixture,
        projection_request(
            Some(&mut original),
            documents,
            Some(&[String::from("body")]),
            ReturnDocuments::Value(false),
        ),
    )
    .unwrap();
    original
}

#[test]
fn rerank_option_projection() {
    let fixture = Fixture::default();
    let documents = [document(r#""a""#), document(r#"{"body":"b"}"#)];
    let mut original = params(r#"{"query":"old","top_n":9,"extension":"yes"}"#);
    let returned = get_optional_rerank_params(
        &fixture,
        projection_request(
            Some(&mut original),
            &documents,
            Some(&[String::from("body")]),
            ReturnDocuments::Value(false),
        ),
    )
    .unwrap();
    assert_eq!(
        returned,
        Params::from([("mapped".to_owned(), Json::Bool(true))])
    );
    assert_eq!(
        original,
        params(
            r#"{"query":"q","top_n":0,"extension":"yes","documents":["a",{"body":"b"}],"return_documents":false,"max_chunks_per_doc":0,"max_tokens_per_doc":3,"instruction":""}"#
        )
    );
}

#[test]
fn the_mapper_sees_the_overwritten_values() {
    let fixture = Fixture::default();
    let documents = [document(r#""a""#), document(r#"{"body":"b"}"#)];
    let _original = project_fixture(&fixture, &documents);
    let calls = fixture.recorded();
    assert_eq!(calls.len(), 1_usize);
    assert_eq!(
        field(&calls[0], "non_default_params"),
        Some(&parsed(
            r#"{"query":"q","top_n":0,"extension":"yes","documents":["a",{"body":"b"}],"return_documents":false,"max_chunks_per_doc":0,"max_tokens_per_doc":3,"instruction":""}"#
        ))
    );
    assert_eq!(
        field(&calls[0], "rank_fields"),
        Some(&parsed(r#"["body"]"#))
    );
    assert_eq!(
        field(&calls[0], "return_documents"),
        Some(&Json::Bool(false))
    );
    assert_eq!(
        field(&calls[0], "custom_llm_provider"),
        Some(&Json::Text(String::from("cohere")))
    );
}

#[test]
fn an_empty_map_is_not_mutated() {
    let fixture = Fixture::default();
    let mut empty = Params::new();
    let documents: [RerankDocument; 0] = [];
    get_optional_rerank_params(
        &fixture,
        projection_request(Some(&mut empty), &documents, None, ReturnDocuments::Null),
    )
    .unwrap();
    assert!(empty.is_empty());
    let calls = fixture.recorded();
    assert_eq!(
        field(&calls[0], "non_default_params"),
        Some(&parsed(r#"{"query":"q","documents":[]}"#))
    );
    assert_eq!(field(&calls[0], "return_documents"), Some(&Json::Null));
}

#[test]
fn an_absent_map_gets_a_new_one_and_the_default_documents_value() {
    let fixture = Fixture::default();
    let documents: [RerankDocument; 0] = [];
    get_optional_rerank_params(
        &fixture,
        projection_request(None, &documents, None, ReturnDocuments::Default),
    )
    .unwrap();
    let calls = fixture.recorded();
    assert_eq!(
        field(&calls[0], "non_default_params"),
        Some(&parsed(
            r#"{"query":"q","documents":[],"return_documents":true}"#
        ))
    );
    assert_eq!(
        field(&calls[0], "return_documents"),
        Some(&Json::Bool(true))
    );
}

#[test]
fn an_existing_rank_fields_entry_is_not_overwritten() {
    let fixture = Fixture::default();
    let documents = [document(r#""a""#)];
    let mut original = params(r#"{"rank_fields":["old"]}"#);
    get_optional_rerank_params(
        &fixture,
        projection_request(
            Some(&mut original),
            &documents,
            Some(&[String::from("body")]),
            ReturnDocuments::Null,
        ),
    )
    .unwrap();
    assert_eq!(
        original.get("rank_fields"),
        Some(&Json::List(vec![Json::Text(String::from("old"))]))
    );
    let calls = fixture.recorded();
    assert_eq!(
        field(&calls[0], "rank_fields"),
        Some(&parsed(r#"["body"]"#))
    );
    assert_eq!(
        field(&calls[0], "non_default_params"),
        Some(&parsed(
            r#"{"rank_fields":["old"],"query":"q","documents":["a"]}"#
        ))
    );
}

#[test]
fn mapper_errors_travel_to_the_caller() {
    let fixture = Fixture {
        calls: Mutex::new(Vec::new()),
        refuse: true,
    };
    let documents: [RerankDocument; 0] = [];
    let outcome = get_optional_rerank_params(
        &fixture,
        projection_request(None, &documents, None, ReturnDocuments::Null),
    );
    assert!(matches!(outcome, Err(FixtureError::Refused)));
}

#[test]
fn rerank_base_defaults() {
    let fixture = Fixture::default();
    assert_eq!(fixture.get_complete_url(None, "m", None), "");
    assert_eq!(fixture.get_complete_url(Some(""), "m", None), "");
    assert_eq!(
        fixture.get_complete_url(Some("https://mock"), "m", None),
        "https://mock"
    );
    assert_eq!(
        fixture
            .transform_rerank_request("m", &Params::new(), &Headers::new(), None)
            .unwrap(),
        Params::new()
    );
    let headers = HeaderMap::from([(
        String::from("retry-after"),
        Json::Number(serde_json::Number::from(2_u8)),
    )]);
    let raised = fixture.get_error_class("slow", 429, headers.clone());
    assert_eq!(
        raised,
        BaseLlmException {
            status_code: 429,
            message: String::from("slow"),
            headers,
        }
    );
    assert_eq!(raised.to_string(), "slow");
}

#[test]
fn the_base_response_transform_returns_the_supplied_response() {
    let fixture = Fixture::default();
    let supplied: RerankResponse = serde_json::from_str(
        r#"{"id":"r","results":[{"index":2,"relevance_score":0.125,"document":{"text":"t"}}]}"#,
    )
    .unwrap();
    let mut logging = RerankLogging::default();
    let raw_response = RerankProviderResponse {
        status_code: 200,
        headers: Headers::new(),
        body: Vec::new(),
    };
    let request_data = Params::new();
    let optional_params = Params::new();
    let litellm_params = Params::new();
    let transformed = fixture
        .transform_rerank_response(
            supplied.clone(),
            ResponseTransform {
                model: "m",
                raw_response: &raw_response,
                logging: &mut logging,
                api_key: None,
                request_data: &request_data,
                optional_params: &optional_params,
                litellm_params: &litellm_params,
            },
        )
        .unwrap();
    assert_eq!(transformed, supplied);
}

#[test]
fn validate_environment_returns_the_supplied_headers() {
    let fixture = Fixture::default();
    let headers = Headers::from([(String::from("accept"), String::from("application/json"))]);
    let validated = fixture
        .validate_environment(EnvironmentRequest {
            headers: &headers,
            model: "m",
            api_key: None,
            optional_params: None,
            litellm_params: None,
        })
        .unwrap();
    assert_eq!(validated, headers);
}

#[rstest]
#[case::priced(
    Some(r#"{"input_cost_per_query":0.125}"#),
    Some(r#"{"search_units":3,"total_tokens":99}"#),
    "0.375"
)]
#[case::zero_units(
    Some(r#"{"input_cost_per_query":0.125}"#),
    Some(r#"{"search_units":0}"#),
    "0"
)]
#[case::negative_units(
    Some(r#"{"input_cost_per_query":0.125}"#),
    Some(r#"{"search_units":-1}"#),
    "-0.125"
)]
#[case::exact_small(
    Some(r#"{"input_cost_per_query":0.000000000000000001}"#),
    Some(r#"{"search_units":3}"#),
    "0.000000000000000003"
)]
#[case::missing_info(None, Some(r#"{"search_units":3}"#), "0")]
#[case::empty_info(Some("{}"), Some(r#"{"search_units":3}"#), "0")]
#[case::null_price(
    Some(r#"{"input_cost_per_query":null}"#),
    Some(r#"{"search_units":3}"#),
    "0"
)]
#[case::other_price_only(
    Some(r#"{"input_cost_per_token":9.0}"#),
    Some(r#"{"search_units":3}"#),
    "0"
)]
#[case::missing_units(Some(r#"{"input_cost_per_query":0.125}"#), None, "0")]
#[case::empty_units(Some(r#"{"input_cost_per_query":0.125}"#), Some("{}"), "0")]
#[case::total_tokens_only(
    Some(r#"{"input_cost_per_query":0.125}"#),
    Some(r#"{"total_tokens":99}"#),
    "0"
)]
#[case::null_units(
    Some(r#"{"input_cost_per_query":0.125}"#),
    Some(r#"{"search_units":null}"#),
    "0"
)]
fn rerank_query_cost(
    #[case] info: Option<&str>,
    #[case] counted: Option<&str>,
    #[case] expected: &str,
) {
    let fixture = Fixture::default();
    let priced = info.map(model_info);
    let units = counted.map(billed_units);
    let (prompt, completion) =
        fixture.calculate_rerank_cost("rerank-model", None, units.as_ref(), priced.as_ref());
    assert_eq!(prompt, decimal(expected));
    assert_eq!(completion, Decimal::ZERO);
}

#[test]
fn an_unrepresentable_cost_saturates() {
    let fixture = Fixture::default();
    let counted = billed_units(r#"{"search_units":2}"#);
    let maximum = RerankModelInfo {
        input_cost_per_query: Some(ExactDecimal(Decimal::MAX)),
    };
    assert_eq!(
        fixture.calculate_rerank_cost("m", None, Some(&counted), Some(&maximum)),
        (Decimal::MAX, Decimal::ZERO)
    );
    let minimum = RerankModelInfo {
        input_cost_per_query: Some(ExactDecimal(Decimal::MIN)),
    };
    assert_eq!(
        fixture.calculate_rerank_cost("m", None, Some(&counted), Some(&minimum)),
        (Decimal::MIN, Decimal::ZERO)
    );
}
