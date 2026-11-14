use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::agent_identity::error::AgentIdentityError;
use crate::agent_policy::inference::{
    InferenceModelKind, ManagedInferenceRequest, managed_inference_request,
};

const MODEL_REQUIRED: &str = "Managed inference requires an explicit or configured model";

fn object(text: &str) -> IndexMap<String, Json> {
    match serde_json::from_str::<Json>(text).unwrap() {
        Json::Object(fields) => fields,
        other @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            panic!("fixture must be an object, got {other:?}");
        }
    }
}

fn model_of(body: &IndexMap<String, Json>) -> String {
    match body.get("model") {
        Some(Json::Text(model)) => model.clone(),
        other => panic!("expected a text model, got {other:?}"),
    }
}

fn failure_of(result: Result<IndexMap<String, Json>, AgentIdentityError>) -> AgentIdentityError {
    match result {
        Ok(body) => panic!("expected a failure, got {body:?}"),
        Err(failure) => failure,
    }
}

fn request<'request>(
    route: &'request str,
    body: &'request IndexMap<String, Json>,
) -> ManagedInferenceRequest<'request> {
    ManagedInferenceRequest {
        route,
        body,
        cli_model: None,
        path_model: None,
        query_model: None,
    }
}

// the probe echoes its arguments so every one the call passes is asserted
fn resolved(
    input: &ManagedInferenceRequest<'_>,
) -> Result<IndexMap<String, Json>, AgentIdentityError> {
    managed_inference_request(input, &|body, cli, endpoint, kind| {
        Some(format!("{kind:?}|{body:?}|{cli:?}|{endpoint:?}"))
    })
}

#[test]
fn route_inference_non_model_route_is_unchanged() {
    let body = object(r#"{"model":"body-model","messages":[]}"#);
    let result = resolved(&request("/v1/agents", &body)).unwrap();
    assert_eq!(result, body);
}

#[test]
fn route_inference_realtime_prefers_query_model() {
    let body = object(r#"{"model":"body-model","stream":true}"#);
    let mut input = request("/realtime", &body);
    input.query_model = Some("query-model");
    let result = resolved(&input).unwrap();
    assert_eq!(model_of(&result), "query-model");
    assert_eq!(result.get("stream"), Some(&Json::Bool(true)));
    assert_eq!(model_of(&body), "body-model");
}

#[test]
fn route_inference_realtime_empty_query_model_falls_back_to_body() {
    let body = object(r#"{"model":"body-model"}"#);
    let mut input = request("/realtime", &body);
    input.query_model = Some("");
    let result = resolved(&input).unwrap();
    assert_eq!(model_of(&result), "body-model");
}

#[test]
fn route_inference_empty_path_model_falls_back_to_query_model() {
    let body = object("{}");
    let mut input = request("/v1/completions", &body);
    input.path_model = Some("");
    input.query_model = Some("query-model");
    let result = resolved(&input).unwrap();
    assert_eq!(
        model_of(&result),
        r#"Completion|None|None|Some("query-model")"#
    );
}

#[test]
fn route_inference_realtime_uses_body_model() {
    let body = object(r#"{"model":"body-model"}"#);
    let result = resolved(&request("/v1/realtime", &body)).unwrap();
    assert_eq!(model_of(&result), "body-model");
}

#[rstest]
#[case::realtime_missing("/realtime", "{}")]
#[case::realtime_empty("/realtime", r#"{"model":""}"#)]
#[case::realtime_non_text("/realtime", r#"{"model":5}"#)]
#[case::realtime_null("/realtime", r#"{"model":null}"#)]
fn route_inference_realtime_requires_model(#[case] route: &str, #[case] input: &str) {
    let body = object(input);
    let failure = failure_of(resolved(&request(route, &body)));
    assert_eq!(failure.status, 403);
    assert_eq!(failure.message, MODEL_REQUIRED);
}

// the resolver reports no configured model for a missing, non-text or empty body model
#[rstest]
#[case::missing("{}")]
#[case::non_text(r#"{"model":5}"#)]
fn route_inference_model_route_requires_model(#[case] input: &str) {
    let body = object(input);
    let failure = failure_of(managed_inference_request(
        &request("/v1/chat/completions", &body),
        &|_, _, _, _| None,
    ));
    assert_eq!(failure.status, 403);
    assert_eq!(failure.message, MODEL_REQUIRED);
}

#[test]
fn route_inference_empty_model_is_required_model() {
    let body = object(r#"{"model":""}"#);
    let failure = failure_of(managed_inference_request(
        &request("/v1/chat/completions", &body),
        &|body_model, _, _, _| body_model.map(str::to_owned),
    ));
    assert_eq!(failure.status, 403);
    assert_eq!(failure.message, MODEL_REQUIRED);
}

#[test]
fn route_inference_completion_passes_body_model_and_kind() {
    let body = object(r#"{"model":"body-model"}"#);
    let result = resolved(&request("/v1/chat/completions", &body)).unwrap();
    assert_eq!(
        model_of(&result),
        r#"Completion|Some("body-model")|None|None"#
    );
}

#[test]
fn route_inference_passes_cli_model() {
    let body = object(r#"{"model":"body-model"}"#);
    let mut input = request("/chat/completions", &body);
    input.cli_model = Some("cli-model");
    let result = resolved(&input).unwrap();
    assert_eq!(
        model_of(&result),
        r#"Completion|Some("body-model")|Some("cli-model")|None"#
    );
}

#[test]
fn route_inference_image_generation_uses_query_model() {
    let body = object("{}");
    let mut input = request("/v1/images/generations", &body);
    input.query_model = Some("query-model");
    let result = resolved(&input).unwrap();
    assert_eq!(
        model_of(&result),
        r#"ImageGeneration|None|None|Some("query-model")"#
    );
}

#[rstest]
#[case::image_edit("/images/edits", InferenceModelKind::ImageEdit)]
#[case::moderations("/v1/moderations", InferenceModelKind::Moderation)]
#[case::transcriptions("/v1/audio/transcriptions", InferenceModelKind::Moderation)]
#[case::speech("/v1/audio/speech", InferenceModelKind::Speech)]
#[case::rerank("/v1/rerank", InferenceModelKind::Body)]
#[case::decisions("/v1/decisions", InferenceModelKind::Body)]
#[case::systemone("/v1/systemone", InferenceModelKind::Body)]
#[case::messages_count_tokens("/v1/messages/count_tokens", InferenceModelKind::Body)]
#[case::messages("/v1/messages", InferenceModelKind::Completion)]
#[case::responses("/v1/responses", InferenceModelKind::Completion)]
fn route_inference_kind_per_route(#[case] route: &str, #[case] kind: InferenceModelKind) {
    let body = object(r#"{"model":"body-model"}"#);
    let result = resolved(&request(route, &body)).unwrap();
    assert_eq!(
        model_of(&result),
        format!(r#"{kind:?}|Some("body-model")|None|None"#)
    );
}

#[test]
fn route_inference_count_tokens_uses_path_kind() {
    let body = object("{}");
    let mut input = request("/v1beta/models/gemini-pro:countTokens", &body);
    input.path_model = Some("gemini-pro");
    let result = resolved(&input).unwrap();
    assert_eq!(model_of(&result), r#"Path|None|None|Some("gemini-pro")"#);
}

#[test]
fn route_inference_path_model_wins_over_query_model() {
    let body = object("{}");
    let mut input = request("/engines/my-deployment/chat/completions", &body);
    input.path_model = Some("deployment-model");
    input.query_model = Some("query-model");
    let result = resolved(&input).unwrap();
    assert_eq!(
        model_of(&result),
        r#"Completion|None|None|Some("deployment-model")"#
    );
}

#[test]
fn route_inference_query_model_fills_endpoint_model() {
    let body = object("{}");
    let mut input = request("/v1/completions", &body);
    input.query_model = Some("query-model");
    let result = resolved(&input).unwrap();
    assert_eq!(
        model_of(&result),
        r#"Completion|None|None|Some("query-model")"#
    );
}

#[test]
fn route_inference_query_model_ignored_where_litellm_ignores_it() {
    let body = object("{}");
    let mut input = request("/v1/responses", &body);
    input.query_model = Some("query-model");
    input.path_model = None;
    let result = resolved(&input).unwrap();
    assert_eq!(model_of(&result), "Completion|None|None|None");
}

#[test]
fn route_inference_templated_path_uses_suffix_kind() {
    let body = object(r#"{"model":"body-model"}"#);
    let mut input = request(
        "/openai/deployments/my-deployment/images/generations",
        &body,
    );
    input.query_model = Some("query-model");
    let result = resolved(&input).unwrap();
    assert_eq!(
        model_of(&result),
        r#"ImageGeneration|Some("body-model")|None|Some("query-model")"#
    );
}

#[rstest]
#[case::embeddings("/v1/embeddings", InferenceModelKind::Completion)]
#[case::image_edits("/v1/images/edits", InferenceModelKind::ImageEdit)]
fn route_inference_query_model_fills_endpoint_for_every_fallback_suffix(
    #[case] route: &str,
    #[case] kind: InferenceModelKind,
) {
    let body = object("{}");
    let mut input = request(route, &body);
    input.query_model = Some("query-model");
    let result = resolved(&input).unwrap();
    assert_eq!(
        model_of(&result),
        format!(r#"{kind:?}|None|None|Some("query-model")"#)
    );
}

#[test]
fn route_inference_does_not_mutate_input() {
    let body = object(r#"{"model":"body-model"}"#);
    let result = resolved(&request("/v1/chat/completions", &body)).unwrap();
    assert_eq!(
        model_of(&result),
        r#"Completion|Some("body-model")|None|None"#
    );
    assert_eq!(model_of(&body), "body-model");
}
