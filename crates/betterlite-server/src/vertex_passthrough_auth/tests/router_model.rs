use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_router::passthrough_selection::error::PassthroughSelectionError;
use serde_json::Number;
use tracing::subscriber::DefaultGuard;

use super::super::router_model::{
    Deployment, LookupError, ResolvedVertexModel, VertexDeploymentLookup, VertexModelRequest,
    resolve_vertex_router_model,
};
use super::support::{LogBuffer, capture_logs, json_text, parameters};

const ROUTE: &str =
    "v1/projects/url-project/locations/us-central1/publishers/google/models/claude:rawPredict";

static NO_NAMES: BTreeSet<String> = BTreeSet::new();
static NO_ENVIRONMENT: BTreeMap<String, String> = BTreeMap::new();

enum Answer {
    Deployment(Deployment),
    Missing,
    Failed,
}

struct FakeLookup {
    answer: Answer,
    seen: Mutex<Vec<String>>,
}

impl FakeLookup {
    fn new(answer: Answer) -> Self {
        Self {
            answer,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    fn record(&self, model: &str) {
        self.seen
            .lock()
            .expect("the seen list is not poisoned")
            .push(model.to_owned());
    }
}

impl VertexDeploymentLookup for FakeLookup {
    fn deployment_for_pass_through(&self, model: &str) -> Result<Option<Deployment>, LookupError> {
        self.record(model);
        match &self.answer {
            Answer::Deployment(deployment) => Ok(Some(deployment.clone())),
            Answer::Missing => Ok(None),
            Answer::Failed => Err(LookupError::from(
                PassthroughSelectionError::NotConfigured {
                    model: model.to_owned(),
                },
            )),
        }
    }
}

fn encoded(route: &str) -> String {
    route.replace('/', "%2F").replace(':', "%3A")
}

fn url_values() -> ResolvedVertexModel {
    ResolvedVertexModel {
        encoded_endpoint: encoded(ROUTE),
        endpoint: ROUTE.to_owned(),
        project: Some("url-project".to_owned()),
        location: Some("us-central1".to_owned()),
        deployment_model_info: None,
    }
}

fn request_on(model_id: &'static str, route: &str) -> VertexModelRequest<'static> {
    VertexModelRequest {
        model_id,
        encoded_endpoint: encoded(route),
        endpoint: route.to_owned(),
        project: Some("url-project".to_owned()),
        location: Some("us-central1".to_owned()),
    }
}

fn request(model_id: &'static str) -> VertexModelRequest<'static> {
    request_on(model_id, ROUTE)
}

fn deployment(params: &[(&str, Json)], model_info: Option<Json>) -> Deployment {
    Deployment {
        litellm_params: parameters(params),
        model_info,
    }
}

fn resolution_context(catalog: &ModelCatalog) -> ResolutionContext<'_> {
    ResolutionContext {
        catalog,
        known_model_names: &NO_NAMES,
        custom_providers: &NO_NAMES,
        environment: &NO_ENVIRONMENT,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    }
}

fn resolved(
    model_id: &'static str,
    answer: Answer,
) -> (ResolvedVertexModel, Vec<String>, LogBuffer, DefaultGuard) {
    let catalog = ModelCatalog::default();
    resolved_with(model_id, answer, &catalog)
}

fn resolved_with(
    model_id: &'static str,
    answer: Answer,
    catalog: &ModelCatalog,
) -> (ResolvedVertexModel, Vec<String>, LogBuffer, DefaultGuard) {
    let (logs, guard) = capture_logs();
    let lookup = FakeLookup::new(answer);
    let lookup_ref: &dyn VertexDeploymentLookup = &lookup;
    let context = resolution_context(catalog);
    let result = resolve_vertex_router_model(request(model_id), Some(lookup_ref), &context);
    (result, lookup.seen(), logs, guard)
}

#[test]
fn no_router_returns_the_inputs_unchanged() {
    let (logs, _guard) = capture_logs();
    let catalog = ModelCatalog::default();
    let result =
        resolve_vertex_router_model(request("claude"), None, &resolution_context(&catalog));
    assert_eq!(result, url_values());
    assert!(!logs.text().contains("get_llm_provider returned"));
}

#[test]
fn a_missing_deployment_returns_the_inputs_unchanged() {
    let (result, seen, logs, _guard) = resolved("claude", Answer::Missing);
    assert_eq!(seen, vec!["claude".to_owned()]);
    assert_eq!(result, url_values());
    assert!(!logs.text().contains("Error resolving"));
}

#[test]
fn a_lookup_error_logs_the_model_and_returns_the_inputs() {
    let (result, seen, logs, _guard) = resolved("claude", Answer::Failed);
    assert_eq!(seen, vec!["claude".to_owned()]);
    assert_eq!(result, url_values());
    let text = logs.text();
    assert!(
        text.contains("Error resolving vertex model from router"),
        "{text}"
    );
    assert!(text.contains("claude"), "{text}");
    assert!(
        text.contains("has no deployments configured with use_in_pass_through=True"),
        "{text}"
    );
}

#[test]
fn the_lookup_error_keeps_the_router_selection_error_as_its_source() {
    let error = LookupError::from(PassthroughSelectionError::NotConfigured {
        model: "claude".to_owned(),
    });
    let source = std::error::Error::source(&error).expect("the router error is the source");
    assert!(source.downcast_ref::<PassthroughSelectionError>().is_some());
}

#[test]
fn router_project_and_location_win_over_the_url() {
    let info = parameters(&[("id", json_text("dep-1"))]);
    let record = deployment(
        &[
            ("vertex_project", json_text("router-project")),
            ("vertex_location", json_text("europe-west4")),
        ],
        Some(Json::Object(info.clone())),
    );
    let (result, _seen, logs, _guard) = resolved("claude", Answer::Deployment(record));
    let expected = ResolvedVertexModel {
        project: Some("router-project".to_owned()),
        location: Some("europe-west4".to_owned()),
        deployment_model_info: Some(info),
        ..url_values()
    };
    assert_eq!(result, expected);
    assert!(!logs.text().contains("get_llm_provider returned"));
}

#[test]
fn empty_router_values_keep_the_url_values() {
    let record = deployment(
        &[
            ("vertex_project", json_text("")),
            ("vertex_location", Json::Null),
        ],
        None,
    );
    let (result, _seen, _logs, _guard) = resolved("claude", Answer::Deployment(record));
    assert_eq!(result, url_values());
}

fn model_info_counts(model_info: Json, expected: Option<&JsonObject>) {
    let record = deployment(
        &[("vertex_project", json_text("router-project"))],
        Some(model_info),
    );
    let (result, _seen, _logs, _guard) = resolved("claude", Answer::Deployment(record));
    assert_eq!(result.deployment_model_info.as_ref(), expected);
    assert_eq!(result.project.as_deref(), Some("router-project"));
}

#[test]
fn model_info_counts_only_when_it_is_an_object() {
    let info = parameters(&[("id", json_text("dep-1"))]);
    model_info_counts(Json::Text("text".to_owned()), None);
    model_info_counts(Json::Bool(true), None);
    model_info_counts(Json::Number(Number::from(7_i64)), None);
    model_info_counts(Json::Null, None);
    model_info_counts(Json::List(Vec::new()), None);
    model_info_counts(Json::Object(info.clone()), Some(&info));
}

#[test]
fn the_provider_prefix_is_stripped_from_every_occurrence() {
    let route =
        "v1/projects/claude/locations/us-central1/publishers/anthropic/models/claude:rawPredict";
    let record = deployment(
        &[(
            "model",
            json_text("vertex_ai/claude-3-5-sonnet-v2@20241022"),
        )],
        None,
    );
    let (logs, _guard) = capture_logs();
    let lookup = FakeLookup::new(Answer::Deployment(record));
    let lookup_ref: &dyn VertexDeploymentLookup = &lookup;
    let catalog = ModelCatalog::default();
    let result = resolve_vertex_router_model(
        request_on("claude", route),
        Some(lookup_ref),
        &resolution_context(&catalog),
    );
    assert_eq!(
        result.endpoint,
        "v1/projects/claude-3-5-sonnet-v2@20241022/locations/us-central1/publishers/anthropic/models/claude-3-5-sonnet-v2@20241022:rawPredict"
    );
    assert_eq!(
        result.encoded_endpoint,
        "v1%2Fprojects%2Fclaude-3-5-sonnet-v2@20241022%2Flocations%2Fus-central1%2Fpublishers%2Fanthropic%2Fmodels%2Fclaude-3-5-sonnet-v2@20241022%3ArawPredict"
    );
    let text = logs.text();
    assert!(text.contains("get_llm_provider returned"), "{text}");
    assert!(text.contains("Resolved router model"), "{text}");
    assert!(text.contains("claude-3-5-sonnet-v2@20241022"), "{text}");
}

#[test]
fn a_bare_catalog_model_resolves_and_replaces_the_url_alias() {
    let info = parameters(&[("id", json_text("dep-1"))]);
    let record = deployment(
        &[("model", json_text("gemini-2.5-flash"))],
        Some(Json::Object(info.clone())),
    );
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let (result, _seen, logs, _guard) =
        resolved_with("claude", Answer::Deployment(record), catalog.as_ref());
    assert_eq!(
        result.endpoint,
        "v1/projects/url-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:rawPredict"
    );
    assert_eq!(result.deployment_model_info, Some(info));
    assert!(logs.text().contains("Resolved router model"));
}

#[test]
fn an_actual_model_equal_to_the_url_id_leaves_the_endpoints_alone() {
    let record = deployment(
        &[
            ("model", json_text("vertex_ai/claude")),
            ("vertex_project", json_text("router-project")),
        ],
        None,
    );
    let (result, _seen, logs, _guard) = resolved("claude", Answer::Deployment(record));
    assert_eq!(result.endpoint, url_values().endpoint);
    assert_eq!(result.encoded_endpoint, url_values().encoded_endpoint);
    assert_eq!(result.project.as_deref(), Some("router-project"));
    let text = logs.text();
    assert!(text.contains("get_llm_provider returned"), "{text}");
    assert!(!text.contains("Resolved router model"), "{text}");
}

#[test]
fn an_empty_actual_model_leaves_the_endpoints_alone() {
    let info = parameters(&[("id", json_text("dep-1"))]);
    let record = deployment(
        &[
            ("model", json_text("vertex_ai/")),
            ("vertex_location", json_text("europe-west4")),
        ],
        Some(Json::Object(info.clone())),
    );
    let (result, _seen, logs, _guard) = resolved("claude", Answer::Deployment(record));
    assert_eq!(result.endpoint, url_values().endpoint);
    assert_eq!(result.encoded_endpoint, url_values().encoded_endpoint);
    assert_eq!(result.location.as_deref(), Some("europe-west4"));
    assert_eq!(result.deployment_model_info, Some(info));
    assert!(!logs.text().contains("Resolved router model"));
}

#[test]
fn a_provider_error_keeps_the_router_project_and_the_url_location() {
    let record = deployment(
        &[
            ("model", json_text("unknown_provider/model-x")),
            ("vertex_project", json_text("router-project")),
        ],
        Some(Json::Object(parameters(&[("id", json_text("dep-1"))]))),
    );
    let (result, _seen, logs, _guard) = resolved("claude", Answer::Deployment(record));
    let expected = ResolvedVertexModel {
        project: Some("router-project".to_owned()),
        ..url_values()
    };
    assert_eq!(result, expected);
    let text = logs.text();
    assert!(
        text.contains("Error resolving vertex model from router"),
        "{text}"
    );
    assert!(text.contains("LLM Provider NOT provided"), "{text}");
    assert!(text.contains("claude"), "{text}");
}

#[test]
fn a_provider_error_keeps_both_router_values() {
    let record = deployment(
        &[
            ("model", json_text("unknown_provider/model-x")),
            ("vertex_project", json_text("router-project")),
            ("vertex_location", json_text("europe-west4")),
        ],
        None,
    );
    let (result, _seen, _logs, _guard) = resolved("claude", Answer::Deployment(record));
    let expected = ResolvedVertexModel {
        project: Some("router-project".to_owned()),
        location: Some("europe-west4".to_owned()),
        ..url_values()
    };
    assert_eq!(result, expected);
}

#[test]
fn the_logs_never_contain_a_router_key() {
    let record = deployment(
        &[
            (
                "model",
                json_text("vertex_ai/claude-3-5-sonnet-v2@20241022"),
            ),
            ("api_key", json_text("sk-router-secret")),
            ("vertex_project", json_text("router-project")),
        ],
        None,
    );
    let (result, _seen, logs, _guard) = resolved("claude", Answer::Deployment(record));
    assert_eq!(result.project.as_deref(), Some("router-project"));
    assert_eq!(
        result.endpoint,
        "v1/projects/url-project/locations/us-central1/publishers/google/models/claude-3-5-sonnet-v2@20241022:rawPredict"
    );
    assert!(!logs.text().contains("sk-router-secret"));
}
