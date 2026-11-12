#![cfg(test)]

use std::sync::Mutex;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankBilledUnits, RerankDocument, RerankResponse,
};
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use thiserror::Error;

use super::{
    BaseLlmException, BaseRerankConfig, EnvironmentRequest, HeaderMap, Headers,
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
    calls: Mutex<Vec<Value>>,
    refuse: bool,
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn optional_text(value: Option<&str>) -> Value {
    match value {
        Some(value) => json!(value),
        None => json!(null),
    }
}

fn optional_number(value: Option<i64>) -> Value {
    match value {
        Some(value) => json!(value),
        None => json!(null),
    }
}

fn optional_bool(value: Option<bool>) -> Value {
    match value {
        Some(value) => json!(value),
        None => json!(null),
    }
}

fn rank_fields(values: Option<&[String]>) -> Value {
    match values {
        Some(values) => json!(values),
        None => json!(null),
    }
}

fn object_of(params: &Params) -> Value {
    serde_json::to_value(Json::Object(
        params
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
    ))
    .unwrap()
}

fn document(value: Value) -> RerankDocument {
    serde_json::from_value(value).unwrap()
}

fn params(value: Value) -> Params {
    serde_json::from_value(value).unwrap()
}

fn record(request: &MapCohereRerankParams<'_>) -> Value {
    json!({
        "non_default_params": object_of(request.non_default_params),
        "model": request.model,
        "drop_params": request.drop_params,
        "query": request.query,
        "documents": request.documents.iter().map(RerankDocument::as_json).collect::<Vec<_>>(),
        "custom_llm_provider": optional_text(request.custom_llm_provider),
        "top_n": optional_number(request.top_n),
        "rank_fields": rank_fields(request.rank_fields),
        "return_documents": optional_bool(request.return_documents),
        "max_chunks_per_doc": optional_number(request.max_chunks_per_doc),
        "max_tokens_per_doc": optional_number(request.max_tokens_per_doc),
        "instruction": optional_text(request.instruction),
    })
}

impl BaseRerankConfig for Fixture {
    type Error = FixtureError;

    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        Ok(request.headers.clone())
    }

    fn get_supported_cohere_rerank_params(&self, _model: &str) -> Vec<String> {
        Vec::new()
    }

    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        if self.refuse {
            return Err(FixtureError::Refused);
        }
        self.calls.lock().unwrap().push(record(&request));
        Ok(Params::from([("mapped".to_owned(), Json::Bool(true))]))
    }
}

#[test]
fn rerank_option_projection() {
    let fixture = Fixture::default();
    let documents = [document(json!("a")), document(json!({"body": "b"}))];
    let mut original = params(json!({"query": "old", "top_n": 9, "extension": "yes"}));
    let returned = get_optional_rerank_params(
        &fixture,
        OptionalRerankParams {
            non_default_params: Some(&mut original),
            model: "rerank-model",
            drop_params: false,
            query: "q",
            documents: &documents,
            custom_llm_provider: Some("cohere"),
            top_n: Some(0),
            rank_fields: Some(&[String::from("body")]),
            return_documents: ReturnDocuments::Value(false),
            max_chunks_per_doc: Some(0),
            max_tokens_per_doc: Some(3),
            instruction: Some(""),
        },
    )
    .unwrap();
    assert_eq!(
        returned,
        Params::from([("mapped".to_owned(), Json::Bool(true))])
    );
    let expected_map = json!({
        "query": "q",
        "top_n": 0,
        "extension": "yes",
        "documents": ["a", {"body": "b"}],
        "return_documents": false,
        "max_chunks_per_doc": 0,
        "max_tokens_per_doc": 3,
        "instruction": "",
    });
    assert_eq!(serde_json::to_value(&original).unwrap(), expected_map);
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(
        calls[0],
        json!({
            "non_default_params": expected_map,
            "model": "rerank-model",
            "drop_params": false,
            "query": "q",
            "documents": ["a", {"body": "b"}],
            "custom_llm_provider": "cohere",
            "top_n": 0,
            "rank_fields": ["body"],
            "return_documents": false,
            "max_chunks_per_doc": 0,
            "max_tokens_per_doc": 3,
            "instruction": "",
        })
    );
}

#[test]
fn an_empty_or_absent_map_gets_a_new_one() {
    let fixture = Fixture::default();
    let mut empty = Params::new();
    let documents: [RerankDocument; 0] = [];
    get_optional_rerank_params(
        &fixture,
        OptionalRerankParams {
            non_default_params: Some(&mut empty),
            model: "m",
            drop_params: true,
            query: "q",
            documents: &documents,
            custom_llm_provider: None,
            top_n: None,
            rank_fields: None,
            return_documents: ReturnDocuments::Null,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
        },
    )
    .unwrap();
    get_optional_rerank_params(
        &fixture,
        OptionalRerankParams {
            non_default_params: None,
            model: "m",
            drop_params: false,
            query: "q",
            documents: &documents,
            custom_llm_provider: None,
            top_n: None,
            rank_fields: None,
            return_documents: ReturnDocuments::Default,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
        },
    )
    .unwrap();
    assert!(empty.is_empty());
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(
        calls[0]["non_default_params"],
        json!({"query": "q", "documents": []})
    );
    assert_eq!(calls[0]["return_documents"], json!(null));
    assert_eq!(
        calls[1]["non_default_params"],
        json!({"query": "q", "documents": [], "return_documents": true})
    );
    assert_eq!(calls[1]["return_documents"], json!(true));
}

#[test]
fn an_existing_rank_fields_entry_is_not_overwritten() {
    let fixture = Fixture::default();
    let documents = [document(json!("a"))];
    let mut original = params(json!({"rank_fields": ["old"]}));
    get_optional_rerank_params(
        &fixture,
        OptionalRerankParams {
            non_default_params: Some(&mut original),
            model: "m",
            drop_params: false,
            query: "q",
            documents: &documents,
            custom_llm_provider: None,
            top_n: None,
            rank_fields: Some(&[String::from("body")]),
            return_documents: ReturnDocuments::Null,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
        },
    )
    .unwrap();
    assert_eq!(
        original.get("rank_fields"),
        Some(&Json::List(vec![Json::Text(String::from("old"))]))
    );
    let calls = fixture.calls.lock().unwrap();
    assert_eq!(calls[0]["rank_fields"], json!(["body"]));
    assert_eq!(
        calls[0]["non_default_params"]["rank_fields"],
        json!(["old"])
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
        OptionalRerankParams {
            non_default_params: None,
            model: "m",
            drop_params: false,
            query: "q",
            documents: &documents,
            custom_llm_provider: None,
            top_n: None,
            rank_fields: None,
            return_documents: ReturnDocuments::Null,
            max_chunks_per_doc: None,
            max_tokens_per_doc: None,
            instruction: None,
        },
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
    let supplied: RerankResponse = serde_json::from_value(json!({
        "id": "r",
        "results": [{"index": 2, "relevance_score": 0.125, "document": {"text": "t"}}],
    }))
    .unwrap();
    let mut logging = RerankLogging::default();
    let raw_response = RerankProviderResponse {
        status_code: 200,
        headers: Headers::new(),
        body: Vec::new(),
    };
    let transformed = fixture
        .transform_rerank_response(
            supplied.clone(),
            ResponseTransform {
                model: "m",
                raw_response: &raw_response,
                logging: &mut logging,
                api_key: None,
                request_data: &Params::new(),
                optional_params: &Params::new(),
                litellm_params: &Params::new(),
            },
        )
        .unwrap();
    assert_eq!(transformed, supplied);
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

fn model_info(value: Value) -> RerankModelInfo {
    serde_json::from_value(value).unwrap()
}

fn billed_units(value: Value) -> RerankBilledUnits {
    serde_json::from_value(value).unwrap()
}

fn decimal(text_value: &str) -> Decimal {
    text_value.parse().unwrap()
}

#[rstest]
#[case::priced(
    json!({"input_cost_per_query": 0.125}),
    json!({"search_units": 3, "total_tokens": 99}),
    decimal("0.375")
)]
#[case::zero_units(
    json!({"input_cost_per_query": 0.125}),
    json!({"search_units": 0}),
    Decimal::ZERO
)]
#[case::negative_units(
    json!({"input_cost_per_query": 0.125}),
    json!({"search_units": -1}),
    decimal("-0.125")
)]
#[case::exact_small(
    json!({"input_cost_per_query": 0.000000000000000001}),
    json!({"search_units": 3}),
    decimal("0.000000000000000003")
)]
#[case::missing_info(Value::Null, json!({"search_units": 3}), Decimal::ZERO)]
#[case::empty_info(json!({}), json!({"search_units": 3}), Decimal::ZERO)]
#[case::null_price(json!({"input_cost_per_query": null}), json!({"search_units": 3}), Decimal::ZERO)]
#[case::other_price_only(json!({"input_cost_per_token": 9.0}), json!({"search_units": 3}), Decimal::ZERO)]
#[case::missing_units(json!({"input_cost_per_query": 0.125}), Value::Null, Decimal::ZERO)]
#[case::empty_units(json!({"input_cost_per_query": 0.125}), json!({}), Decimal::ZERO)]
#[case::total_tokens_only(
    json!({"input_cost_per_query": 0.125}),
    json!({"total_tokens": 99}),
    Decimal::ZERO
)]
#[case::null_units(json!({"input_cost_per_query": 0.125}), json!({"search_units": null}), Decimal::ZERO)]
fn rerank_query_cost(#[case] info: Value, #[case] units: Value, #[case] expected: Decimal) {
    let fixture = Fixture::default();
    let units = if units.is_null() {
        None
    } else {
        Some(billed_units(units))
    };
    let info = if info.is_null() {
        None
    } else {
        Some(model_info(info))
    };
    let (prompt, completion) =
        fixture.calculate_rerank_cost("rerank-model", None, units.as_ref(), info.as_ref());
    assert_eq!(prompt, expected);
    assert_eq!(completion, Decimal::ZERO);
}

#[test]
fn an_unrepresentable_cost_saturates() {
    let fixture = Fixture::default();
    let info = RerankModelInfo {
        input_cost_per_query: Some(Decimal::MAX),
    };
    let units = billed_units(json!({"search_units": 2}));
    assert_eq!(
        fixture.calculate_rerank_cost("m", None, Some(&units), Some(&info)),
        (Decimal::MAX, Decimal::ZERO)
    );
    let negative = RerankModelInfo {
        input_cost_per_query: Some(Decimal::MIN),
    };
    assert_eq!(
        fixture.calculate_rerank_cost("m", None, Some(&units), Some(&negative)),
        (Decimal::MIN, Decimal::ZERO)
    );
}
