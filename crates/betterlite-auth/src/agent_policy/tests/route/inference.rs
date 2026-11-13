use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::agent_identity::error::AgentIdentityError;
use crate::agent_policy::inference::{
    InferenceModelKind, InferenceModelSettings, ManagedInferenceRequest, managed_inference_request,
    resolve_inference_model,
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

fn settings(
    completion: Option<&str>,
    image_generation: Option<&str>,
    moderation: Option<&str>,
) -> InferenceModelSettings {
    InferenceModelSettings {
        completion_model: completion.map(str::to_owned),
        image_generation_model: image_generation.map(str::to_owned),
        moderation_model: moderation.map(str::to_owned),
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
    settings: &'request InferenceModelSettings,
) -> ManagedInferenceRequest<'request> {
    ManagedInferenceRequest {
        route,
        body,
        settings,
        cli_model: None,
        path_model: None,
        query_model: None,
    }
}

#[test]
fn route_inference_non_model_route_is_unchanged() {
    let body = object(r#"{"model":"body-model","messages":[]}"#);
    let defaults = settings(None, None, None);
    let result = managed_inference_request(&request("/v1/agents", &body, &defaults)).unwrap();
    assert_eq!(result, body);
}

#[test]
fn route_inference_realtime_prefers_query_model() {
    let body = object(r#"{"model":"body-model","stream":true}"#);
    let defaults = settings(None, None, None);
    let mut input = request("/realtime", &body, &defaults);
    input.query_model = Some("query-model");
    let result = managed_inference_request(&input).unwrap();
    assert_eq!(model_of(&result), "query-model");
    assert_eq!(result.get("stream"), Some(&Json::Bool(true)));
    assert_eq!(model_of(&body), "body-model");
}

#[test]
fn route_inference_realtime_uses_body_model() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(None, None, None);
    let result = managed_inference_request(&request("/v1/realtime", &body, &defaults)).unwrap();
    assert_eq!(model_of(&result), "body-model");
}

#[rstest]
#[case::realtime_missing("/realtime", "{}")]
#[case::realtime_empty("/realtime", r#"{"model":""}"#)]
#[case::realtime_non_text("/realtime", r#"{"model":5}"#)]
#[case::realtime_null("/realtime", r#"{"model":null}"#)]
#[case::completions_missing("/v1/chat/completions", "{}")]
#[case::completions_empty("/v1/chat/completions", r#"{"model":""}"#)]
#[case::completions_non_text("/v1/chat/completions", r#"{"model":5}"#)]
fn route_inference_requires_model(#[case] route: &str, #[case] input: &str) {
    let body = object(input);
    let defaults = settings(None, None, None);
    let failure = failure_of(managed_inference_request(&request(route, &body, &defaults)));
    assert_eq!(failure.status, 403);
    assert_eq!(failure.message, MODEL_REQUIRED);
}

#[test]
fn route_inference_completion_prefers_settings() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(Some("configured-model"), None, None);
    let result =
        managed_inference_request(&request("/v1/chat/completions", &body, &defaults)).unwrap();
    assert_eq!(model_of(&result), "configured-model");
}

#[test]
fn route_inference_completion_uses_body_model() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(None, None, None);
    let result =
        managed_inference_request(&request("/chat/completions", &body, &defaults)).unwrap();
    assert_eq!(model_of(&result), "body-model");
}

#[test]
fn route_inference_image_generation_prefers_cli_model() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(Some("completion-model"), Some("image-model"), None);
    let mut input = request("/v1/images/generations", &body, &defaults);
    input.cli_model = Some("cli-model");
    let result = managed_inference_request(&input).unwrap();
    assert_eq!(model_of(&result), "cli-model");
}

#[test]
fn route_inference_image_generation_uses_query_model() {
    let body = object("{}");
    let defaults = settings(None, Some("image-model"), None);
    let mut input = request("/v1/images/generations", &body, &defaults);
    input.query_model = Some("query-model");
    let result = managed_inference_request(&input).unwrap();
    assert_eq!(model_of(&result), "query-model");
}

#[test]
fn route_inference_image_edit_prefers_completion_model() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(Some("completion-model"), Some("image-model"), None);
    let result = managed_inference_request(&request("/images/edits", &body, &defaults)).unwrap();
    assert_eq!(model_of(&result), "completion-model");
}

#[test]
fn route_inference_moderation_uses_moderation_model() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(Some("completion-model"), None, Some("moderation-model"));
    let result = managed_inference_request(&request("/v1/moderations", &body, &defaults)).unwrap();
    assert_eq!(model_of(&result), "moderation-model");
    let transcription =
        managed_inference_request(&request("/v1/audio/transcriptions", &body, &defaults)).unwrap();
    assert_eq!(model_of(&transcription), "moderation-model");
}

#[test]
fn route_inference_speech_uses_body_model() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(Some("completion-model"), None, Some("moderation-model"));
    let result = managed_inference_request(&request("/v1/audio/speech", &body, &defaults)).unwrap();
    assert_eq!(model_of(&result), "body-model");
}

#[rstest]
#[case::rerank("/v1/rerank")]
#[case::decisions("/v1/decisions")]
#[case::systemone("/v1/systemone")]
#[case::messages_count_tokens("/v1/messages/count_tokens")]
fn route_inference_body_kind_uses_body_model(#[case] route: &str) {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(Some("completion-model"), None, None);
    let result = managed_inference_request(&request(route, &body, &defaults)).unwrap();
    assert_eq!(model_of(&result), "body-model");
}

#[test]
fn route_inference_count_tokens_uses_path_model() {
    let body = object("{}");
    let defaults = settings(Some("completion-model"), None, None);
    let mut input = request("/v1beta/models/gemini-pro:countTokens", &body, &defaults);
    input.path_model = Some("gemini-pro");
    let result = managed_inference_request(&input).unwrap();
    assert_eq!(model_of(&result), "gemini-pro");
}

#[test]
fn route_inference_path_model_wins_over_query_model() {
    let body = object("{}");
    let defaults = settings(None, None, None);
    let mut input = request("/engines/my-deployment/chat/completions", &body, &defaults);
    input.path_model = Some("deployment-model");
    input.query_model = Some("query-model");
    let result = managed_inference_request(&input).unwrap();
    assert_eq!(model_of(&result), "deployment-model");
}

#[test]
fn route_inference_query_model_fills_endpoint_model() {
    let body = object("{}");
    let defaults = settings(None, None, None);
    let mut input = request("/v1/completions", &body, &defaults);
    input.query_model = Some("query-model");
    let result = managed_inference_request(&input).unwrap();
    assert_eq!(model_of(&result), "query-model");
}

#[test]
fn route_inference_templated_model_path_uses_body_settings() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(Some("configured-model"), None, None);
    let result = managed_inference_request(&request(
        "/openai/deployments/my-deployment/embeddings",
        &body,
        &defaults,
    ))
    .unwrap();
    assert_eq!(model_of(&result), "configured-model");
}

#[test]
fn route_inference_does_not_mutate_input() {
    let body = object(r#"{"model":"body-model"}"#);
    let defaults = settings(Some("configured-model"), None, None);
    let result =
        managed_inference_request(&request("/v1/chat/completions", &body, &defaults)).unwrap();
    assert_eq!(model_of(&result), "configured-model");
    assert_eq!(model_of(&body), "body-model");
}

#[rstest]
#[case::image_generation(InferenceModelKind::ImageGeneration, "cli", "endpoint", "body", "cli")]
#[case::image_edit(InferenceModelKind::ImageEdit, "cli", "endpoint", "body", "completion")]
#[case::moderation(InferenceModelKind::Moderation, "cli", "endpoint", "body", "cli")]
#[case::speech(InferenceModelKind::Speech, "cli", "endpoint", "body", "cli")]
#[case::body(InferenceModelKind::Body, "cli", "endpoint", "body", "body")]
#[case::path(InferenceModelKind::Path, "cli", "endpoint", "body", "endpoint")]
fn resolve_inference_model_kind_order(
    #[case] kind: InferenceModelKind,
    #[case] cli: &str,
    #[case] endpoint: &str,
    #[case] body: &str,
    #[case] expected: &str,
) {
    let defaults = settings(Some("completion"), Some("image"), Some("moderation"));
    let resolved = resolve_inference_model(Some(body), &defaults, Some(cli), Some(endpoint), kind);
    assert_eq!(resolved, Some(expected));
}

#[test]
fn resolve_inference_model_body_falls_back() {
    let defaults = settings(None, None, None);
    let resolved = resolve_inference_model(
        Some("body"),
        &defaults,
        None,
        None,
        InferenceModelKind::Completion,
    );
    assert_eq!(resolved, Some("body"));
}
