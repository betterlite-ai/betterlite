#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::catalog::{
    CatalogError, Collaborators, DeploymentRecord, ManagedIdDecoders, ManagedIdError,
    ManagedIdField, ModelCatalog,
};
use super::managed::{
    cache_prediction_model_candidates, extract_models_from_managed_resource_id,
    get_cache_prediction_deployments,
};
use super::model::get_model_from_request;
use super::sources::{
    DispatchContext, RequestSnapshot, extract_model_candidates_from_request,
    request_dispatched_to_pass_through_endpoint, request_dispatched_to_provider_pass_through,
};
use super::{
    RequestedModels, append_model_candidates, dedupe_model_candidates, format_model_candidates,
    route_matches_any_marker, route_uses_model_routing_sources,
};

fn object(text: &str) -> JsonObject {
    serde_json::from_str(text).expect("the fixture parses")
}

#[derive(Debug, Default)]
struct FakeCatalog {
    deployments: Vec<DeploymentRecord>,
    model_names: BTreeSet<String>,
    resolved: BTreeMap<String, String>,
    failing: bool,
}

impl ModelCatalog for FakeCatalog {
    fn resolve_model_name_from_model_id(
        &self,
        model_id: &str,
    ) -> Result<Option<String>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(self.resolved.get(model_id).cloned())
    }

    fn deployment(&self, deployment_id: &str) -> Result<Option<DeploymentRecord>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(self
            .deployments
            .iter()
            .find(|deployment| deployment.deployment_id.as_deref() == Some(deployment_id))
            .cloned())
    }

    fn model_names(&self) -> Result<BTreeSet<String>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(self.model_names.clone())
    }

    fn model_list(&self) -> Result<Vec<DeploymentRecord>, CatalogError> {
        if self.failing {
            return Err(CatalogError::InvalidDeployment);
        }
        Ok(self.deployments.clone())
    }
}

#[derive(Debug)]
struct StubDecoders;

impl ManagedIdDecoders for StubDecoders {
    fn legacy_file_batch(
        &self,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }

    fn unified(
        &self,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }

    fn video_character(
        &self,
        _resource_id: &str,
        _field: ManagedIdField,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
struct RecordingDecoders {
    ids_seen: Arc<Mutex<Vec<String>>>,
}

impl ManagedIdDecoders for RecordingDecoders {
    fn legacy_file_batch(
        &self,
        resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        self.ids_seen
            .lock()
            .expect("the call log is not poisoned")
            .push(resource_id.to_owned());
        Ok(())
    }

    fn unified(
        &self,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }

    fn video_character(
        &self,
        _resource_id: &str,
        _field: ManagedIdField,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }
}

fn deployment(model_name: &str, team_id: Option<&str>) -> DeploymentRecord {
    DeploymentRecord {
        model_name: model_name.to_owned(),
        custom_llm_provider: None,
        model: None,
        team_id: team_id.map(str::to_owned),
        deployment_id: Some(format!("{model_name}-id")),
    }
}

fn candidates(
    route: &str,
    body: &JsonObject,
    headers: &JsonObject,
    query: &JsonObject,
    catalog: &dyn ModelCatalog,
    decoders: &dyn ManagedIdDecoders,
) -> Vec<String> {
    let path_params = BTreeSet::new();
    let request = RequestSnapshot {
        route,
        body,
        headers: Some(headers),
        query: Some(query),
        dispatch: DispatchContext {
            endpoint_marker: None,
            path_params: &path_params,
        },
        team_id: None,
    };
    let collaborators = Collaborators {
        catalog: Some(catalog),
        decoders,
    };
    extract_model_candidates_from_request(&request, &collaborators)
        .expect("the catalog lookup succeeds")
}

fn simple_candidates(route: &str, body: &JsonObject) -> Vec<String> {
    candidates(
        route,
        body,
        &JsonObject::new(),
        &JsonObject::new(),
        &FakeCatalog::default(),
        &StubDecoders,
    )
}

fn resolve(route: &str, body: &JsonObject, catalog: Option<&dyn ModelCatalog>) -> RequestedModels {
    let headers = JsonObject::new();
    let query = JsonObject::new();
    let path_params = BTreeSet::new();
    let request = RequestSnapshot {
        route,
        body,
        headers: Some(&headers),
        query: Some(&query),
        dispatch: DispatchContext {
            endpoint_marker: None,
            path_params: &path_params,
        },
        team_id: None,
    };
    let decoders = StubDecoders;
    let collaborators = Collaborators {
        catalog,
        decoders: &decoders,
    };
    get_model_from_request(&request, &collaborators).expect("the catalog lookup succeeds")
}

#[test]
fn scalar_sequence_comma_values() {
    let body = object(r#"{"model": "gpt-4, claude , ,o3"}"#);
    assert_eq!(
        simple_candidates("/v1/chat/completions", &body),
        ["gpt-4", "claude", "o3"]
    );
    let listed = object(r#"{"model": ["a, b", "c", "d"]}"#);
    assert_eq!(
        simple_candidates("/v1/chat/completions", &listed),
        ["a", "b", "c", "d"]
    );
}

#[test]
fn trim_empty_none_values() {
    let body = object(r#"{"model": "  a  , , b "}"#);
    assert_eq!(simple_candidates("/v1/chat/completions", &body), ["a", "b"]);
    let empty = object(r#"{"model": null}"#);
    assert!(simple_candidates("/v1/chat/completions", &empty).is_empty());
    let listed = object(r#"{"model": ["", " ", "x"]}"#);
    assert_eq!(simple_candidates("/v1/chat/completions", &listed), ["x"]);
}

#[test]
fn nonstring_values_stringified() {
    let number = object(r#"{"model": 42}"#);
    assert_eq!(simple_candidates("/v1/chat/completions", &number), ["42"]);
    let flag = object(r#"{"model": true}"#);
    assert_eq!(simple_candidates("/v1/chat/completions", &flag), ["True"]);
    let listed = object(r#"{"model": [1, false, "x"]}"#);
    assert_eq!(
        simple_candidates("/v1/chat/completions", &listed),
        ["1", "False", "x"]
    );
}

#[test]
fn first_occurrence_dedup() {
    let candidates = ["a", "b", "a", "c", "b"]
        .iter()
        .map(|model| (*model).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(dedupe_model_candidates(&candidates), ["a", "b", "c"]);
}

#[rstest]
#[case(
    r#"{"X-Litellm-Model": "casefold", "x-litellm-model": "exact"}"#,
    "exact"
)]
#[case(r#"{"X-LITELLM-MODEL": "upper"}"#, "upper")]
fn exact_key_before_casefold(#[case] headers: &str, #[case] expected: &str) {
    let headers = object(headers);
    let found = candidates(
        "/files",
        &JsonObject::new(),
        &headers,
        &JsonObject::new(),
        &FakeCatalog::default(),
        &StubDecoders,
    );
    assert_eq!(found, [expected]);
}

#[rstest]
#[case("/V1/FILES/list", true, true)]
#[case("/v1/skills", false, true)]
#[case("/v1/vector_stores", false, true)]
#[case("/v1/chat/completions", false, false)]
fn marker_substring_casefold(
    #[case] route: &str,
    #[case] matches_files: bool,
    #[case] routing_source: bool,
) {
    assert_eq!(route_matches_any_marker(route, &["/files"]), matches_files);
    assert_eq!(route_uses_model_routing_sources(route), routing_source);
}

#[rstest]
#[case("/files", true)]
#[case("/batches/1", true)]
#[case("/vector_stores", true)]
#[case("/v1/chat/completions", false)]
fn target_model_route_matrix(#[case] route: &str, #[case] appends_target: bool) {
    let body = object(r#"{"model": "body-model", "target_model_names": "target-model"}"#);
    let found = simple_candidates(route, &body);
    if appends_target {
        assert_eq!(found, ["body-model", "target-model"]);
    } else {
        assert_eq!(found, ["body-model"]);
    }
}

#[test]
fn body_model_first() {
    let body = object(r#"{"model": "body-model", "target_model_names": "target-model"}"#);
    assert_eq!(
        simple_candidates("/files", &body),
        ["body-model", "target-model"]
    );
}

#[test]
fn falsey_body_model_appends_target_model_names() {
    let body = object(r#"{"model": null, "target_model_names": ["a", "b"]}"#);
    assert_eq!(simple_candidates("/v1/chat/completions", &body), ["a", "b"]);
}

#[rstest]
#[case("/realtime/client_secrets", Some("session-model"))]
#[case("/realtime/calls", Some("session-model"))]
#[case("/v1/chat/completions", None)]
fn realtime_session_model(#[case] route: &str, #[case] expected: Option<&str>) {
    let body = object(r#"{"session": {"model": "session-model"}}"#);
    let found = simple_candidates(route, &body);
    assert_eq!(found.first().map(String::as_str), expected);
}

#[test]
fn eval_completion_model() {
    let body = object(r#"{"completion": {"model": "completion-model"}}"#);
    assert_eq!(simple_candidates("/evals", &body), ["completion-model"]);
    assert!(simple_candidates("/v1/chat/completions", &body).is_empty());
}

#[test]
fn query_before_header() {
    let query = object(r#"{"MODEL": "from-query"}"#);
    let headers = object(r#"{"x-litellm-model": "from-header"}"#);
    let found = candidates(
        "/files",
        &JsonObject::new(),
        &headers,
        &query,
        &FakeCatalog::default(),
        &StubDecoders,
    );
    assert_eq!(found, ["from-query", "from-header"]);
}

#[rstest]
#[case("/files", true)]
#[case("/fine_tuning", true)]
#[case("/vector_stores", false)]
fn query_target_route_matrix(#[case] route: &str, #[case] appends_target: bool) {
    let query = object(r#"{"target_model_names": "query-target"}"#);
    let found = candidates(
        route,
        &JsonObject::new(),
        &JsonObject::new(),
        &query,
        &FakeCatalog::default(),
        &StubDecoders,
    );
    assert_eq!(found.len(), usize::from(appends_target));
}

#[test]
fn unrelated_route_ignores_ids() {
    let body = object(
        r#"{"file_id": "file-1", "batch_id": "b", "video_id": "video_1", "character_id": "character_1"}"#,
    );
    assert!(simple_candidates("/v1/chat/completions", &body).is_empty());
}

#[test]
fn each_managed_id_field_order() {
    let mut fields = JsonObject::new();
    for field in super::MODEL_ROUTING_ID_FIELDS {
        fields.insert(field.to_owned(), Json::Text(format!("{field}-value")));
    }
    let decoders = RecordingDecoders::default();
    candidates(
        "/files",
        &fields,
        &JsonObject::new(),
        &JsonObject::new(),
        &FakeCatalog::default(),
        &decoders,
    );
    let seen = decoders
        .ids_seen
        .lock()
        .expect("the call log is not poisoned")
        .clone();
    let expected: Vec<String> = super::MODEL_ROUTING_ID_FIELDS
        .iter()
        .map(|field| format!("{field}-value"))
        .collect();
    assert_eq!(seen, expected);
}

#[test]
fn prediction_missing_router_ids() {
    let body = object(r#"{"current_deployment_id": "a", "candidate_deployment_id": "b"}"#);
    assert_eq!(
        cache_prediction_model_candidates(&body, None, None),
        Ok(Vec::new())
    );
    let missing = object(r#"{"current_deployment_id": 1, "candidate_deployment_id": "b"}"#);
    let catalog = FakeCatalog::default();
    assert_eq!(
        cache_prediction_model_candidates(&missing, Some(&catalog), None),
        Ok(Vec::new())
    );
    assert_eq!(
        cache_prediction_model_candidates(&JsonObject::new(), Some(&catalog), None),
        Ok(Vec::new())
    );
}

#[test]
fn prediction_missing_deployment() {
    let catalog = FakeCatalog {
        deployments: vec![deployment("known", None)],
        ..FakeCatalog::default()
    };
    assert_eq!(
        get_cache_prediction_deployments("known-id", "gone-id", Some(&catalog), None),
        Ok(None)
    );
}

#[rstest]
#[case(Some("other-team"), None, false)]
#[case(Some("other-team"), Some("team"), false)]
#[case(Some("team"), Some("other-team"), false)]
#[case(None, Some("team"), false)]
#[case(None, None, true)]
#[case(Some("team"), Some("team"), true)]
fn prediction_team_scope(
    #[case] current_team: Option<&str>,
    #[case] team_id: Option<&str>,
    #[case] allowed: bool,
) {
    let catalog = FakeCatalog {
        deployments: vec![
            deployment("current", current_team),
            deployment("candidate", None),
        ],
        ..FakeCatalog::default()
    };
    let found =
        get_cache_prediction_deployments("current-id", "candidate-id", Some(&catalog), team_id);
    let names = found
        .expect("the catalog lookup succeeds")
        .map(|(current, candidate)| (current.model_name, candidate.model_name));
    assert_eq!(names.is_some(), allowed);
    if let Some((current, candidate)) = names {
        assert_eq!(
            (current.as_str(), candidate.as_str()),
            ("current", "candidate")
        );
    }
}

#[test]
fn prediction_global_or_same_team() {
    let body = object(
        r#"{"current_deployment_id": "current-id", "candidate_deployment_id": "candidate-id"}"#,
    );
    let catalog = FakeCatalog {
        deployments: vec![
            deployment("current", None),
            deployment("candidate", Some("team")),
        ],
        ..FakeCatalog::default()
    };
    assert_eq!(
        cache_prediction_model_candidates(&body, Some(&catalog), Some("team")),
        Ok(vec!["current".to_owned(), "candidate".to_owned()])
    );
}

#[test]
fn prediction_ignores_body() {
    let body = object(
        r#"{"model": "body-model", "target_model_names": "target", "current_deployment_id": "current-id", "candidate_deployment_id": "candidate-id"}"#,
    );
    let catalog = FakeCatalog {
        deployments: vec![deployment("current", None), deployment("candidate", None)],
        ..FakeCatalog::default()
    };
    assert_eq!(
        resolve("/cost/predict-cache", &body, Some(&catalog)),
        RequestedModels::Many(vec!["current".to_owned(), "candidate".to_owned()])
    );
}

#[test]
fn zero_one_many_format() {
    assert_eq!(format_model_candidates(Vec::new()), RequestedModels::None);
    assert_eq!(
        format_model_candidates(vec!["one".to_owned()]),
        RequestedModels::One("one".to_owned())
    );
    assert_eq!(
        format_model_candidates(vec!["one".to_owned(), "two".to_owned()]),
        RequestedModels::Many(vec!["one".to_owned(), "two".to_owned()])
    );
}

#[test]
fn marker_exact_true_skips() {
    let body = object(r#"{"model": "body-model"}"#);
    let path_params = BTreeSet::new();
    let decoders = StubDecoders;
    let catalog = FakeCatalog::default();
    let collaborators = Collaborators {
        catalog: Some(&catalog),
        decoders: &decoders,
    };
    for marker in [Some(false), None] {
        let kept = RequestSnapshot {
            route: "/openai/deployments/path-model/chat/completions",
            body: &body,
            headers: None,
            query: None,
            dispatch: DispatchContext {
                endpoint_marker: marker,
                path_params: &path_params,
            },
            team_id: None,
        };
        assert_eq!(
            get_model_from_request(&kept, &collaborators),
            Ok(RequestedModels::One("body-model".to_owned()))
        );
    }
    let skipped = RequestSnapshot {
        route: "/openai/deployments/path-model/chat/completions",
        body: &body,
        headers: None,
        query: None,
        dispatch: DispatchContext {
            endpoint_marker: Some(true),
            path_params: &path_params,
        },
        team_id: None,
    };
    assert_eq!(
        get_model_from_request(&skipped, &collaborators),
        Ok(RequestedModels::None)
    );
}

#[test]
fn provider_endpoint_presence() {
    let params = BTreeSet::from(["endpoint".to_owned()]);
    let present = DispatchContext {
        endpoint_marker: None,
        path_params: &params,
    };
    assert!(request_dispatched_to_provider_pass_through(&present));
    let empty = BTreeSet::new();
    let absent = DispatchContext {
        endpoint_marker: None,
        path_params: &empty,
    };
    assert!(!request_dispatched_to_provider_pass_through(&absent));
    assert!(!request_dispatched_to_pass_through_endpoint(&absent));
}

#[rstest]
#[case("/azure/chat/gpt-4/chat/completions", Some("gpt-4"))]
#[case("/azure_ai/chat/gpt-4/chat/completions", Some("gpt-4"))]
#[case("/azure/chat/unknown/chat/completions", None)]
#[case("/v1/chat/completions", None)]
fn azure_openai_path_fallback(#[case] route: &str, #[case] expected: Option<&str>) {
    let catalog = FakeCatalog {
        model_names: BTreeSet::from(["gpt-4".to_owned()]),
        ..FakeCatalog::default()
    };
    let expected = expected.map_or(RequestedModels::None, |name| {
        RequestedModels::One(name.to_owned())
    });
    assert_eq!(resolve(route, &JsonObject::new(), Some(&catalog)), expected);
}

#[rstest]
#[case("/v1beta/models/gemini-2.0-flash:generateContent", "gemini-2.0-flash")]
#[case(
    "/v1beta/models/bedrock/claude-sonnet-3.7:generateContent",
    "bedrock/claude-sonnet-3.7"
)]
#[case("/beta/models/m:streamGenerateContent", "m")]
#[case("/models/custom/ns/model:streamGenerateContent", "custom/ns/model")]
fn google_slash_model(#[case] route: &str, #[case] expected: &str) {
    assert_eq!(
        resolve(route, &JsonObject::new(), None),
        RequestedModels::One(expected.to_owned())
    );
}

#[test]
fn vertex_path_exclusion() {
    assert_eq!(
        resolve(
            "/vertex_ai/v1/models/gemini-1.5-pro",
            &JsonObject::new(),
            None
        ),
        RequestedModels::One("gemini-1.5-pro".to_owned())
    );
    let body = object(r#"{"model": "body-model"}"#);
    assert_eq!(
        resolve("/vertex_ai/v1/models/path-model", &body, None),
        RequestedModels::One("body-model".to_owned())
    );
}

#[test]
fn count_tokens_keeps_body() {
    let body = object(r#"{"model": "body-model"}"#);
    assert_eq!(
        resolve("/bedrock/model/m/count_tokens", &body, None),
        RequestedModels::One("body-model".to_owned())
    );
    assert_eq!(
        resolve("/bedrock/model/m/count-tokens", &JsonObject::new(), None),
        RequestedModels::None
    );
}

#[test]
fn azure_catalog_path_overrides() {
    let catalog = FakeCatalog {
        model_names: BTreeSet::from(["gpt-4".to_owned()]),
        ..FakeCatalog::default()
    };
    let body = object(r#"{"model": "body-model"}"#);
    assert_eq!(
        resolve("/azure/chat/gpt-4/chat/completions", &body, Some(&catalog)),
        RequestedModels::One("gpt-4".to_owned())
    );
}

#[test]
fn nvidia_longest_exclusive_group() {
    let mut shared_nim = deployment("meta", None);
    shared_nim.custom_llm_provider = Some("nvidia_nim".to_owned());
    let mut shared_openai = deployment("meta", None);
    shared_openai.custom_llm_provider = Some("openai".to_owned());
    let mut longer = deployment("meta/llama-3", None);
    longer.custom_llm_provider = Some("nvidia_nim".to_owned());
    let catalog = FakeCatalog {
        deployments: vec![shared_nim, shared_openai, longer],
        ..FakeCatalog::default()
    };
    assert_eq!(
        resolve(
            "/nvidia_nim/meta/llama-3/v1/chat/completions",
            &JsonObject::new(),
            Some(&catalog)
        ),
        RequestedModels::One("meta/llama-3".to_owned())
    );
}

#[test]
fn router_absent_keeps_body() {
    let body = object(r#"{"model": "body-model"}"#);
    assert_eq!(
        resolve("/azure/chat/gpt-4/chat/completions", &body, None),
        RequestedModels::One("body-model".to_owned())
    );
    assert_eq!(
        resolve("/nvidia_nim/meta/v1/chat/completions", &body, None),
        RequestedModels::One("body-model".to_owned())
    );
}

#[test]
fn append_candidates_keeps_commas_and_empties_out() {
    let mut found = Vec::new();
    append_model_candidates(&mut found, &Json::Text(" a, ,b ".to_owned()));
    assert_eq!(found, ["a", "b"]);
}

#[derive(Debug)]
struct FailingSecondFamily;

impl ManagedIdDecoders for FailingSecondFamily {
    fn legacy_file_batch(
        &self,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        candidates.push("partial".to_owned());
        Ok(())
    }

    fn unified(
        &self,
        _resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Err(ManagedIdError::Decoder)
    }

    fn video_character(
        &self,
        _resource_id: &str,
        _field: ManagedIdField,
        _catalog: Option<&dyn ModelCatalog>,
        _candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        Ok(())
    }
}

#[test]
fn family_error_retains_partial() {
    let value = Json::Text("any-id".to_owned());
    assert_eq!(
        extract_models_from_managed_resource_id(&value, "file_id", None, &FailingSecondFamily),
        ["partial"]
    );
}
