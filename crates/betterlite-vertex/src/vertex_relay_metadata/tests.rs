#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::cost_usage::CostUsage;
use rstest::rstest;
use rust_decimal::Decimal;

use super::error::VertexRelayError;
use super::interactions::{
    InteractionsPassthroughInput, InteractionsPassthroughOutcome, interactions_handler,
};
use super::payload::{ResponseLoggingPayloadInput, response_logging_payload};
use super::routes::{is_interactions_route, is_vertex_interactions_route};
use super::types::{
    CompletionCost, CompletionCostRequest, CompletionResponse, CustomLlmProvider, LoggingObject,
    ModelCallDetails, ResponseLoggingKwargs, VertexPublisher,
};
use super::url_models::{custom_provider_from_url, extract_model_from_url, publisher_from_url};

struct FakeCost {
    cost: Option<Decimal>,
}

impl CompletionCost for FakeCost {
    fn completion_cost(&self, _request: &CompletionCostRequest<'_>) -> Option<Decimal> {
        self.cost
    }
}

struct MatchingCost {
    model: &'static str,
    provider: CustomLlmProvider,
    vertex_location: Option<&'static str>,
    requires_usage: bool,
    cost: Decimal,
}

impl CompletionCost for MatchingCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Option<Decimal> {
        let model_matches = request.model == self.model;
        let provider_matches = request.custom_llm_provider == self.provider;
        let location_matches = request.vertex_location == self.vertex_location;
        let usage_matches = !self.requires_usage || request.response.usage.is_some();
        (model_matches && provider_matches && location_matches && usage_matches)
            .then_some(self.cost)
    }
}

fn logging_object() -> LoggingObject {
    LoggingObject {
        litellm_call_id: "call-123".to_owned(),
        model: None,
        custom_llm_provider: None,
        model_call_details: ModelCallDetails {
            model: None,
            custom_llm_provider: None,
            response_cost: None,
        },
    }
}

fn empty_kwargs() -> ResponseLoggingKwargs {
    ResponseLoggingKwargs {
        response_cost: None,
        model: None,
        custom_llm_provider: None,
    }
}

fn cost_usage(prompt_tokens: i64, completion_tokens: i64, total_tokens: i64) -> CostUsage {
    CostUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

fn completion_response(model: Option<&str>, usage: Option<CostUsage>) -> CompletionResponse {
    CompletionResponse {
        id: Some("original-id".to_owned()),
        model: model.map(str::to_owned),
        usage,
    }
}

fn run_interactions(
    body: &str,
    request_model: Option<&str>,
    cost: &dyn CompletionCost,
) -> (InteractionsPassthroughOutcome, LoggingObject) {
    let mut logging = logging_object();
    let outcome = interactions_handler(
        cost,
        InteractionsPassthroughInput {
            response_body: body.as_bytes(),
            request_model,
            logging: &mut logging,
            kwargs: empty_kwargs(),
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: None,
        },
    )
    .expect("valid interactions response body");
    (outcome, logging)
}

#[rstest]
#[case::absolute(
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/interactions",
    true
)]
#[case::trailing_slash(
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/interactions/",
    true
)]
#[case::query_ignored(
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/interactions?alt=json",
    true
)]
#[case::plain_path("/v1/projects/proj/locations/us-central1/interactions", true)]
#[case::last_segment_parameters(
    "https://host/v1/projects/proj/locations/us-central1/interactions;x",
    true
)]
#[case::extra_segment("/v1/projects/proj/locations/us-central1/interactions/extra", false)]
#[case::missing_location("/v1/projects/proj/locations/interactions", false)]
#[case::empty_location("/v1/projects/proj/locations//interactions", false)]
#[case::generate_content(
    "/v1/projects/proj/locations/us-central1/publishers/google/models/x:generateContent",
    false
)]
fn vertex_interactions_route_detection(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_vertex_interactions_route(route), expected);
}

#[rstest]
#[case::suffix("https://host/v1/interactions", true)]
#[case::trailing_slash("https://host/v1/interactions/", true)]
#[case::many_trailing_slashes("https://host/v1/interactions///", true)]
#[case::query_ignored("https://host/v1/interactions?alt=json", true)]
#[case::last_segment_parameters("https://host/v1/interactions;x", true)]
#[case::relative_path("/v1/interactions", true)]
#[case::extra_segment("https://host/v1/interactions/extra", false)]
#[case::similar_suffix("/v1/interactions2", false)]
#[case::empty("", false)]
fn interactions_route_detection(#[case] route: &str, #[case] expected: bool) {
    assert_eq!(is_interactions_route(route), expected);
}

#[rstest]
#[case::publisher_path(
    "https://host/v1/projects/p/locations/l/publishers/google/models/gemini-2.5-flash:generateContent",
    "gemini-2.5-flash"
)]
#[case::method_suffix("/models/foo:predict", "foo")]
#[case::slash_kept("/models/a/b", "a/b")]
#[case::query_kept("/models/foo?alt=json", "foo?alt=json")]
#[case::no_models_segment("/v1/projects/p/locations/l", "unknown")]
#[case::empty_name("/models/:predict", "unknown")]
fn url_model_extraction(#[case] url: &str, #[case] expected: &str) {
    assert_eq!(extract_model_from_url(url), expected);
}

#[rstest]
#[case::mistralai(
    "https://host/v1/projects/p/locations/l/publishers/mistralai/models/mistral-large",
    Some(VertexPublisher::MistralAi)
)]
#[case::anthropic(
    "https://host/v1/projects/p/locations/l/publishers/anthropic/models/claude",
    Some(VertexPublisher::Anthropic)
)]
#[case::ai21(
    "https://host/v1/projects/p/locations/l/publishers/ai21/models/jamba",
    Some(VertexPublisher::Ai21)
)]
#[case::openapi(
    "https://host/v1/projects/p/locations/l/endpoints/openapi/models/foo:predict",
    Some(VertexPublisher::OpenApi)
)]
#[case::google_publisher(
    "https://host/v1/projects/p/locations/l/publishers/google/models/gemini",
    None
)]
#[case::unrelated("https://host/v1/projects/p/locations/l", None)]
fn provider_detection(#[case] url: &str, #[case] expected: Option<VertexPublisher>) {
    assert_eq!(publisher_from_url(url), expected);
}

#[rstest]
#[case::gemini(
    "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent",
    CustomLlmProvider::Gemini
)]
#[case::gemini_subdomain(
    "https://us-central1-generativelanguage.googleapis.com/v1beta/models/gemini:generateContent",
    CustomLlmProvider::Gemini
)]
#[case::vertex(
    "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/google/models/gemini:generateContent",
    CustomLlmProvider::VertexAi
)]
#[case::relative(
    "/v1/projects/p/locations/us-central1/interactions",
    CustomLlmProvider::VertexAi
)]
#[case::lookalike_host(
    "https://generativelanguage.googleapis.com.evil.example/v1beta/models/gemini:generateContent",
    CustomLlmProvider::VertexAi
)]
fn custom_provider_from_url_identifies_gemini_hosts(
    #[case] url: &str,
    #[case] expected: CustomLlmProvider,
) {
    assert_eq!(custom_provider_from_url(url), expected);
}

#[rstest]
#[case::response_model_wins(
    r#"{"model":"response-model","usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
    Some("request-model"),
    Some("response-model")
)]
#[case::empty_response_model_falls_back(
    r#"{"model":"","usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
    Some("request-model"),
    Some("request-model")
)]
#[case::null_response_model_falls_back(
    r#"{"model":null,"usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
    Some("request-model"),
    Some("request-model")
)]
#[case::non_string_response_model_falls_back(
    r#"{"model":7,"usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
    Some("request-model"),
    Some("request-model")
)]
#[case::empty_request_model_is_absent(
    r#"{"model":"","usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
    Some(""),
    None
)]
#[case::neither_model_is_present(
    r#"{"usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
    None,
    None
)]
fn model_extraction_prefers_response_then_request(
    #[case] body: &str,
    #[case] request_model: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let cost = FakeCost {
        cost: Some(Decimal::new(125, 2)),
    };
    let (outcome, logging) = run_interactions(body, request_model, &cost);
    let model = outcome
        .result
        .as_ref()
        .and_then(|response| response.model.as_deref());
    assert_eq!(model, expected);
    if expected.is_none() {
        assert_eq!(outcome.kwargs, empty_kwargs());
        assert_eq!(logging.custom_llm_provider, None);
        assert_eq!(logging.model_call_details, ModelCallDetails::default());
    }
}

#[rstest]
#[case::empty_array_body("[]")]
#[case::non_empty_array_body(r#"["m",{"total_input_tokens":3}]"#)]
#[case::invalid_json("not json")]
fn model_extraction_rejects_a_non_object_body(#[case] body: &str) {
    let cost = FakeCost {
        cost: Some(Decimal::new(125, 2)),
    };
    let error = interactions_handler(
        &cost,
        InteractionsPassthroughInput {
            response_body: body.as_bytes(),
            request_model: None,
            logging: &mut logging_object(),
            kwargs: empty_kwargs(),
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: None,
        },
    )
    .expect_err("a non-object body is a decode failure");
    assert!(matches!(error, VertexRelayError::ResponseBody { .. }));
}

#[test]
fn interactions_usage_is_projected_for_a_valid_object() {
    let cost = FakeCost {
        cost: Some(Decimal::new(125, 2)),
    };
    let body = r#"{"model":"model-name","usage":{"total_input_tokens":10,"total_output_tokens":4,"total_tokens":14}}"#;
    let (outcome, _logging) = run_interactions(body, None, &cost);
    let response = outcome.result.expect("a priced response");
    assert_eq!(response.usage, Some(cost_usage(10, 4, 14)));
}

#[rstest]
#[case::prompt_tokens_disqualify(
    r#"{"model":"m","usage":{"prompt_tokens":1,"total_input_tokens":3}}"#
)]
#[case::input_tokens_disqualify(
    r#"{"model":"m","usage":{"input_tokens":1,"total_output_tokens":3}}"#
)]
#[case::no_total_counters(r#"{"model":"m","usage":{"total_tokens":3}}"#)]
#[case::null_usage(r#"{"model":"m","usage":null}"#)]
#[case::empty_list_usage(r#"{"model":"m","usage":[]}"#)]
#[case::list_usage(r#"{"model":"m","usage":[3,2]}"#)]
#[case::missing_usage(r#"{"model":"m"}"#)]
fn interactions_usage_rejects_a_non_interactions_object(#[case] body: &str) {
    let cost = FakeCost {
        cost: Some(Decimal::new(125, 2)),
    };
    let (outcome, logging) = run_interactions(body, None, &cost);
    assert_eq!(outcome.result, None);
    assert_eq!(outcome.kwargs, empty_kwargs());
    assert_eq!(logging.model, None);
    assert_eq!(logging.custom_llm_provider, None);
    assert_eq!(logging.model_call_details, ModelCallDetails::default());
}

#[test]
fn interactions_usage_transformation_errors_surface_as_errors() {
    let cost = FakeCost {
        cost: Some(Decimal::new(125, 2)),
    };
    let body = r#"{"model":"m","usage":{"total_input_tokens":1,"input_tokens_by_modality":[{"modality":"text","tokens":9223372036854775807},{"modality":"text","tokens":1}]}}"#;
    let error = interactions_handler(
        &cost,
        InteractionsPassthroughInput {
            response_body: body.as_bytes(),
            request_model: None,
            logging: &mut logging_object(),
            kwargs: empty_kwargs(),
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: None,
        },
    )
    .expect_err("an overflowing modality sum is a projection failure");
    assert!(matches!(error, VertexRelayError::Usage { .. }));
}

#[test]
fn logging_payload_enriches_kwargs_and_the_logging_object() {
    let cost = MatchingCost {
        model: "request-model",
        provider: CustomLlmProvider::VertexAi,
        vertex_location: Some("us-central1"),
        requires_usage: true,
        cost: Decimal::new(125, 2),
    };
    let mut response = completion_response(Some("response-model"), Some(cost_usage(3, 2, 5)));
    let mut logging = logging_object();
    let kwargs = response_logging_payload(
        &cost,
        ResponseLoggingPayloadInput {
            response: &mut response,
            model: "request-model",
            kwargs: empty_kwargs(),
            logging: &mut logging,
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: Some("us-central1"),
        },
    );
    assert_eq!(kwargs.response_cost, Some(Decimal::new(125, 2)));
    assert_eq!(kwargs.model.as_deref(), Some("request-model"));
    assert_eq!(kwargs.custom_llm_provider, None);
    assert_eq!(response.id.as_deref(), Some("call-123"));
    assert_eq!(response.model.as_deref(), Some("response-model"));
    assert_eq!(logging.model.as_deref(), Some("response-model"));
    assert_eq!(
        logging.model_call_details.model.as_deref(),
        Some("response-model")
    );
    assert_eq!(
        logging.model_call_details.custom_llm_provider,
        Some(CustomLlmProvider::VertexAi)
    );
    assert_eq!(logging.model_call_details.response_cost, None);
}

#[rstest]
#[case::missing_model(None)]
#[case::empty_model(Some(""))]
fn logging_payload_falls_back_to_the_request_model(#[case] response_model: Option<&str>) {
    let cost = FakeCost {
        cost: Some(Decimal::new(125, 2)),
    };
    let mut response = completion_response(response_model, None);
    let mut logging = logging_object();
    let _kwargs = response_logging_payload(
        &cost,
        ResponseLoggingPayloadInput {
            response: &mut response,
            model: "request-model",
            kwargs: empty_kwargs(),
            logging: &mut logging,
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: None,
        },
    );
    assert_eq!(logging.model.as_deref(), Some("request-model"));
    assert_eq!(
        logging.model_call_details.model.as_deref(),
        Some("request-model")
    );
}

#[test]
fn logging_payload_from_the_interactions_handler_adds_the_provider() {
    let cost = MatchingCost {
        model: "response-model",
        provider: CustomLlmProvider::Gemini,
        vertex_location: None,
        requires_usage: true,
        cost: Decimal::new(125, 2),
    };
    let mut logging = logging_object();
    let outcome = interactions_handler(
        &cost,
        InteractionsPassthroughInput {
            response_body: br#"{"model":"response-model","usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
            request_model: None,
            logging: &mut logging,
            kwargs: empty_kwargs(),
            custom_llm_provider: CustomLlmProvider::Gemini,
            vertex_location: None,
        },
    )
    .expect("valid interactions response body");
    assert_eq!(
        outcome.kwargs.custom_llm_provider,
        Some(CustomLlmProvider::Gemini)
    );
    assert_eq!(outcome.kwargs.model.as_deref(), Some("response-model"));
    assert_eq!(outcome.kwargs.response_cost, Some(Decimal::new(125, 2)));
    assert_eq!(logging.custom_llm_provider, Some(CustomLlmProvider::Gemini));
    assert_eq!(logging.model.as_deref(), Some("response-model"));
    assert_eq!(
        logging.model_call_details.custom_llm_provider,
        Some(CustomLlmProvider::Gemini)
    );
    assert_eq!(logging.model_call_details.response_cost, None);
    assert_eq!(
        outcome.result.expect("a priced response").id.as_deref(),
        Some("call-123")
    );
}

#[test]
fn cost_calculation_passes_model_provider_and_location_to_the_trait() {
    let cost = MatchingCost {
        model: "request-model",
        provider: CustomLlmProvider::VertexAi,
        vertex_location: Some("us-central1"),
        requires_usage: true,
        cost: Decimal::new(125, 2),
    };
    let mut response = completion_response(Some("response-model"), Some(cost_usage(3, 2, 5)));
    let mut logging = logging_object();
    let kwargs = response_logging_payload(
        &cost,
        ResponseLoggingPayloadInput {
            response: &mut response,
            model: "request-model",
            kwargs: empty_kwargs(),
            logging: &mut logging,
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: Some("us-central1"),
        },
    );
    assert_eq!(kwargs.response_cost, Some(Decimal::new(125, 2)));
}

#[test]
fn cost_calculation_without_a_location_does_not_invent_one() {
    let cost = MatchingCost {
        model: "request-model",
        provider: CustomLlmProvider::VertexAi,
        vertex_location: None,
        requires_usage: true,
        cost: Decimal::new(125, 2),
    };
    let mut response = completion_response(Some("response-model"), Some(cost_usage(3, 2, 5)));
    let mut logging = logging_object();
    let kwargs = response_logging_payload(
        &cost,
        ResponseLoggingPayloadInput {
            response: &mut response,
            model: "request-model",
            kwargs: empty_kwargs(),
            logging: &mut logging,
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: None,
        },
    );
    assert_eq!(kwargs.response_cost, Some(Decimal::new(125, 2)));
}

#[test]
fn cost_calculation_failure_records_zero_and_keeps_the_row() {
    let cost = FakeCost { cost: None };
    let mut response = completion_response(Some("response-model"), Some(cost_usage(3, 2, 5)));
    let mut logging = logging_object();
    let kwargs = response_logging_payload(
        &cost,
        ResponseLoggingPayloadInput {
            response: &mut response,
            model: "request-model",
            kwargs: empty_kwargs(),
            logging: &mut logging,
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: None,
        },
    );
    assert_eq!(kwargs.response_cost, Some(Decimal::ZERO));
    assert_eq!(kwargs.model.as_deref(), Some("request-model"));
    assert_eq!(logging.model.as_deref(), Some("response-model"));
    assert_eq!(logging.model_call_details.response_cost, None);
}

#[test]
fn cost_calculation_without_usage_is_unpriceable() {
    let cost = MatchingCost {
        model: "request-model",
        provider: CustomLlmProvider::VertexAi,
        vertex_location: None,
        requires_usage: true,
        cost: Decimal::new(125, 2),
    };
    let mut response = completion_response(Some("response-model"), None);
    let mut logging = logging_object();
    let kwargs = response_logging_payload(
        &cost,
        ResponseLoggingPayloadInput {
            response: &mut response,
            model: "request-model",
            kwargs: empty_kwargs(),
            logging: &mut logging,
            custom_llm_provider: CustomLlmProvider::VertexAi,
            vertex_location: None,
        },
    );
    assert_eq!(kwargs.response_cost, Some(Decimal::ZERO));
    assert_eq!(logging.model_call_details.response_cost, None);
}
