#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::catalog::{
    CatalogError, Collaborators, DeploymentRecord, EmptyCatalog, ManagedIdDecoders, ManagedIdError,
    ManagedIdFamily, ModelCatalog, RequestModelResolver,
};
use super::managed::{
    managed_resource_models, prediction_candidates, prediction_deployments,
    resolve_model_id_with_router,
};
use super::model::get_model_from_request;
use super::sources::{
    DispatchContext, RequestSnapshot, is_pass_through_dispatch, is_provider_pass_through,
    request_model_candidates,
};
use super::{
    MODEL_ROUTING_ROUTE_MARKERS, RequestedModels, append_model_candidates, dedupe_model_candidates,
    format_model_candidates, is_falsey, push_optional_model, route_matches_any_marker,
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
    fn resolve_model_name(&self, model_id: &str) -> Result<Option<String>, CatalogError> {
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

#[derive(Debug, Default)]
struct RecordingDecoders {
    legacy_ids: Arc<Mutex<Vec<String>>>,
    family_calls: Arc<Mutex<Vec<String>>>,
    hint_models: Vec<String>,
    fails_unified: bool,
}

impl RecordingDecoders {
    fn with_hints(hint_models: &[&str]) -> Self {
        Self {
            hint_models: hint_models
                .iter()
                .map(|model| (*model).to_owned())
                .collect(),
            ..Self::default()
        }
    }

    fn failing_unified() -> Self {
        Self {
            fails_unified: true,
            ..Self::default()
        }
    }
}

impl ManagedIdDecoders for RecordingDecoders {
    fn decode_family(
        &self,
        family: ManagedIdFamily,
        resource_id: &str,
        _catalog: Option<&dyn ModelCatalog>,
        candidates: &mut Vec<String>,
    ) -> Result<(), ManagedIdError> {
        self.family_calls
            .lock()
            .map_err(|_poisoned| ManagedIdError::Decoder)?
            .push(format!("{family:?}:{resource_id}"));
        if family == ManagedIdFamily::LegacyFileBatch {
            self.legacy_ids
                .lock()
                .map_err(|_poisoned| ManagedIdError::Decoder)?
                .push(resource_id.to_owned());
        }
        candidates.extend_from_slice(&self.hint_models);
        if !self.fails_unified {
            return Ok(());
        }
        match family {
            ManagedIdFamily::LegacyFileBatch => candidates.push("partial".to_owned()),
            ManagedIdFamily::Unified => return Err(ManagedIdError::Decoder),
            ManagedIdFamily::Video | ManagedIdFamily::Character => {}
        }
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
    collaborators: &Collaborators<'_>,
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
    request_model_candidates(&request, collaborators).expect("the catalog lookup succeeds")
}

fn simple_candidates(route: &str, body: &JsonObject) -> Vec<String> {
    let catalog = FakeCatalog::default();
    let decoders = RecordingDecoders::default();
    let collaborators = Collaborators {
        catalog: Some(&catalog),
        decoders: &decoders,
    };
    candidates(
        route,
        body,
        &JsonObject::new(),
        &JsonObject::new(),
        &collaborators,
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
    let decoders = RecordingDecoders::default();
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
fn exact_key_before_casefold(#[case] header_json: &str, #[case] expected_model: &str) {
    let headers = object(header_json);
    let found = candidates(
        "/files",
        &JsonObject::new(),
        &headers,
        &JsonObject::new(),
        &Collaborators {
            catalog: Some(&FakeCatalog::default()),
            decoders: &RecordingDecoders::default(),
        },
    );
    assert_eq!(found, [expected_model]);
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
    assert_eq!(
        route_matches_any_marker(route, &MODEL_ROUTING_ROUTE_MARKERS),
        routing_source
    );
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
        &Collaborators {
            catalog: Some(&FakeCatalog::default()),
            decoders: &RecordingDecoders::default(),
        },
    );
    assert_eq!(found, ["from-query", "from-header"]);
}

#[rstest]
#[case("/files", &["query-target"])]
#[case("/fine_tuning", &["query-target"])]
#[case("/vector_stores", &[])]
fn query_target_route_matrix(#[case] route: &str, #[case] expected: &[&str]) {
    let query = object(r#"{"target_model_names": "query-target"}"#);
    let found = candidates(
        route,
        &JsonObject::new(),
        &JsonObject::new(),
        &query,
        &Collaborators {
            catalog: Some(&FakeCatalog::default()),
            decoders: &RecordingDecoders::default(),
        },
    );
    assert_eq!(found, expected.to_vec());
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
    let fields: JsonObject = super::MODEL_ROUTING_ID_FIELDS
        .iter()
        .map(|field| ((*field).to_owned(), Json::Text(format!("{field}-value"))))
        .collect();
    let decoders = RecordingDecoders::default();
    candidates(
        "/files",
        &fields,
        &JsonObject::new(),
        &JsonObject::new(),
        &Collaborators {
            catalog: Some(&FakeCatalog::default()),
            decoders: &decoders,
        },
    );
    let seen = decoders
        .legacy_ids
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
    assert_eq!(prediction_candidates(&body, None, None), Ok(Vec::new()));
    let missing = object(r#"{"current_deployment_id": 1, "candidate_deployment_id": "b"}"#);
    let catalog = FakeCatalog::default();
    assert_eq!(
        prediction_candidates(&missing, Some(&catalog), None),
        Ok(Vec::new())
    );
    assert_eq!(
        prediction_candidates(&JsonObject::new(), Some(&catalog), None),
        Ok(Vec::new())
    );
}

#[test]
fn prediction_missing_deployment() {
    let catalog = FakeCatalog {
        deployments: vec![deployment("known", None)],
        model_names: BTreeSet::new(),
        resolved: BTreeMap::new(),
        failing: false,
    };
    assert_eq!(
        prediction_deployments("known-id", "gone-id", Some(&catalog), None),
        Ok(None)
    );
}

#[rstest]
#[case(Some("other-team"), None, false)]
#[case(Some("other-team"), Some("team"), false)]
#[case(Some("team"), Some("other-team"), false)]
#[case(None, Some("team"), true)]
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
        model_names: BTreeSet::new(),
        resolved: BTreeMap::new(),
        failing: false,
    };
    let found = prediction_deployments("current-id", "candidate-id", Some(&catalog), team_id);
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
        model_names: BTreeSet::new(),
        resolved: BTreeMap::new(),
        failing: false,
    };
    assert_eq!(
        prediction_candidates(&body, Some(&catalog), Some("team")),
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
        model_names: BTreeSet::new(),
        resolved: BTreeMap::new(),
        failing: false,
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

#[rstest]
#[case::false_marker(Some(false), false)]
#[case::absent_marker(None, false)]
#[case::true_marker(Some(true), true)]
fn marker_exact_true_skips(#[case] marker: Option<bool>, #[case] skips: bool) {
    let body = object(r#"{"model": "body-model"}"#);
    let path_params = BTreeSet::new();
    let decoders = RecordingDecoders::default();
    let catalog = FakeCatalog::default();
    let collaborators = Collaborators {
        catalog: Some(&catalog),
        decoders: &decoders,
    };
    let request = RequestSnapshot {
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
    let expected = if skips {
        RequestedModels::None
    } else {
        RequestedModels::One("body-model".to_owned())
    };
    assert_eq!(
        get_model_from_request(&request, &collaborators),
        Ok(expected)
    );
}

#[test]
fn provider_endpoint_presence() {
    let params = BTreeSet::from(["endpoint".to_owned()]);
    let present = DispatchContext {
        endpoint_marker: None,
        path_params: &params,
    };
    assert!(is_provider_pass_through(&present));
    let empty = BTreeSet::new();
    let absent = DispatchContext {
        endpoint_marker: None,
        path_params: &empty,
    };
    assert!(!is_provider_pass_through(&absent));
    assert!(!is_pass_through_dispatch(&absent));
}

#[rstest]
#[case("/azure/chat/gpt-4/chat/completions", Some("gpt-4"))]
#[case("/azure_ai/chat/gpt-4/chat/completions", Some("gpt-4"))]
#[case("/azure/chat/unknown/chat/completions", None)]
#[case("/v1/chat/completions", None)]
#[case("/openai/deployments/dep-model/chat/completions", Some("dep-model"))]
#[case("/openai/deployments/dep-model", Some("dep-model"))]
#[case("/openai/deployments//chat/completions", None)]
#[case("/OpenAI/deployments/dep-model/chat/completions", None)]
#[case("/AZURE/chat/gpt-4/chat/completions", Some("gpt-4"))]
fn azure_openai_path_fallback(#[case] route: &str, #[case] expected: Option<&str>) {
    let catalog = FakeCatalog {
        deployments: Vec::new(),
        model_names: BTreeSet::from(["gpt-4".to_owned()]),
        resolved: BTreeMap::new(),
        failing: false,
    };
    let wanted = expected.map_or(RequestedModels::None, |name| {
        RequestedModels::One(name.to_owned())
    });
    assert_eq!(resolve(route, &JsonObject::new(), Some(&catalog)), wanted);
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
        deployments: Vec::new(),
        model_names: BTreeSet::from(["gpt-4".to_owned()]),
        resolved: BTreeMap::new(),
        failing: false,
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
        model_names: BTreeSet::new(),
        resolved: BTreeMap::new(),
        failing: false,
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

#[test]
fn family_error_retains_partial() {
    let value = Json::Text("any-id".to_owned());
    let decoders = RecordingDecoders::failing_unified();
    assert_eq!(
        managed_resource_models(&value, "file_id", None, &decoders),
        ["partial"]
    );
}

#[test]
fn prediction_other_team_denied() {
    let body = object(
        r#"{"current_deployment_id": "current-id", "candidate_deployment_id": "candidate-id"}"#,
    );
    let catalog = FakeCatalog {
        deployments: vec![
            deployment("current", None),
            deployment("candidate", Some("other-team")),
        ],
        model_names: BTreeSet::new(),
        resolved: BTreeMap::new(),
        failing: false,
    };
    assert_eq!(
        prediction_candidates(&body, Some(&catalog), Some("team")),
        Ok(Vec::new())
    );
    assert_eq!(
        prediction_candidates(&body, Some(&catalog), None),
        Ok(Vec::new())
    );
}

#[test]
fn bedrock_path_overrides_body() {
    let body = object(r#"{"model": "body-model"}"#);
    assert_eq!(
        resolve("/bedrock/model/anthropic.claude-v2/invoke", &body, None),
        RequestedModels::One("anthropic.claude-v2".to_owned())
    );
    assert_eq!(
        resolve("/BEDROCK/model/anthropic.claude-v2/invoke", &body, None),
        RequestedModels::One("anthropic.claude-v2".to_owned())
    );
    assert_eq!(
        resolve("/bedrock/not-a-model-path", &body, None),
        RequestedModels::One("body-model".to_owned())
    );
    assert_eq!(
        resolve("/bedrock/anthropic.claude-v2/invoke", &body, None),
        RequestedModels::One("body-model".to_owned())
    );
}

#[test]
fn azure_prefix_is_stripped_before_matching() {
    let catalog = FakeCatalog {
        deployments: Vec::new(),
        model_names: BTreeSet::from(["azure".to_owned()]),
        resolved: BTreeMap::new(),
        failing: false,
    };
    let body = object(r#"{"model": "body-model"}"#);
    assert_eq!(
        resolve("/azure/chat/completions", &body, Some(&catalog)),
        RequestedModels::One("body-model".to_owned())
    );
    assert_eq!(
        resolve("/azure_ai/chat/completions", &body, Some(&catalog)),
        RequestedModels::One("body-model".to_owned())
    );
}

#[rstest]
#[case("/v1beta/models/:generateContent")]
#[case("/v1/models/gpt-4")]
#[case("/v1beta/models/no-colon")]
#[case("/models/no-colon")]
fn google_path_without_model_is_ignored(#[case] route: &str) {
    assert_eq!(
        resolve(route, &JsonObject::new(), None),
        RequestedModels::None
    );
}

#[rstest]
#[case(Json::Null)]
#[case(Json::Bool(false))]
#[case(Json::Text(String::new()))]
#[case(Json::List(Vec::new()))]
#[case(Json::Object(JsonObject::new()))]
#[case(serde_json::from_str("0").expect("zero parses"))]
#[case(serde_json::from_str("0.0").expect("zero parses"))]
fn falsey_model_appends_target_model_names(#[case] model: Json) {
    assert!(is_falsey(&model));
    let mut body = object(r#"{"target_model_names": "t"}"#);
    body.insert("model".to_owned(), model);
    assert_eq!(
        simple_candidates("/v1/chat/completions", &body).last(),
        Some(&"t".to_owned())
    );
}

#[rstest]
#[case(Json::Bool(true))]
#[case(Json::Text("m".to_owned()))]
#[case(serde_json::from_str("1").expect("one parses"))]
#[case(serde_json::from_str("0.5").expect("fraction parses"))]
fn truthy_model_skips_target_model_names(#[case] model: Json) {
    assert!(!is_falsey(&model));
    let mut body = object(r#"{"target_model_names": "t"}"#);
    body.insert("model".to_owned(), model);
    assert!(!simple_candidates("/v1/chat/completions", &body).contains(&"t".to_owned()));
}

#[test]
fn empty_catalog_knows_nothing() {
    let catalog = EmptyCatalog;
    assert_eq!(catalog.resolve_model_name("m"), Ok(None));
    assert_eq!(catalog.deployment("d"), Ok(None));
    assert_eq!(catalog.model_names(), Ok(BTreeSet::new()));
    assert_eq!(catalog.model_list(), Ok(Vec::new()));
}

#[rstest]
#[case::named_id("named", "group")]
#[case::blank_resolution("blank", "blank")]
#[case::unknown_id("other", "other")]
fn model_id_resolution_keeps_original_unless_named(#[case] model_id: &str, #[case] expected: &str) {
    let catalog = FakeCatalog {
        deployments: Vec::new(),
        model_names: BTreeSet::new(),
        resolved: BTreeMap::from([
            ("named".to_owned(), "group".to_owned()),
            ("blank".to_owned(), String::new()),
        ]),
        failing: false,
    };
    assert_eq!(
        resolve_model_id_with_router(Some(model_id.to_owned()), Some(&catalog)),
        Some(expected.to_owned())
    );
    assert_eq!(resolve_model_id_with_router(None, Some(&catalog)), None);
    assert_eq!(
        resolve_model_id_with_router(Some("named".to_owned()), None),
        Some("named".to_owned())
    );
}

#[test]
fn vertex_later_models_segment_after_empty_one() {
    assert_eq!(
        resolve(
            "/vertex_ai/models/:x/models/m:generate",
            &JsonObject::new(),
            None
        ),
        RequestedModels::One("m".to_owned())
    );
}

#[rstest]
#[case::plain_route(None, RequestedModels::One("body-model".to_owned()))]
#[case::marked_endpoint(Some(true), RequestedModels::None)]
fn custom_collision_still_checks(#[case] marker: Option<bool>, #[case] expected: RequestedModels) {
    let body = object(r#"{"model": "body-model"}"#);
    let path_params = BTreeSet::new();
    let collaborators = Collaborators {
        catalog: None,
        decoders: &RecordingDecoders::default(),
    };
    let request = RequestSnapshot {
        route: "/v1/chat/completions",
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
        get_model_from_request(&request, &collaborators),
        Ok(expected)
    );
}

#[test]
fn managed_id_hints_reach_the_candidate_list() {
    let body = object(r#"{"file_id": "file-123"}"#);
    let catalog = FakeCatalog::default();
    let decoders = RecordingDecoders::with_hints(&["from-id"]);
    let collaborators = Collaborators {
        catalog: Some(&catalog),
        decoders: &decoders,
    };
    assert_eq!(
        candidates(
            "/files",
            &body,
            &JsonObject::new(),
            &JsonObject::new(),
            &collaborators,
        ),
        ["from-id"]
    );
}

#[test]
fn optional_model_hint_appends_only_when_present() {
    let mut candidates = Vec::new();
    push_optional_model(&mut candidates, Some("model".to_owned()));
    push_optional_model(&mut candidates, None);
    assert_eq!(candidates, ["model"]);
}

#[test]
fn video_and_character_ids_try_their_family() {
    let body = object(r#"{"video_id": "video-1", "character_id": "character-1"}"#);
    let catalog = FakeCatalog::default();
    let decoders = RecordingDecoders::default();
    let collaborators = Collaborators {
        catalog: Some(&catalog),
        decoders: &decoders,
    };
    assert!(
        candidates(
            "/files",
            &body,
            &JsonObject::new(),
            &JsonObject::new(),
            &collaborators,
        )
        .is_empty()
    );
    let calls = decoders
        .family_calls
        .lock()
        .expect("the recording mutex is not poisoned")
        .clone();
    assert_eq!(
        calls,
        [
            "LegacyFileBatch:video-1",
            "Unified:video-1",
            "Video:video-1",
            "LegacyFileBatch:character-1",
            "Unified:character-1",
            "Character:character-1",
        ]
    );
}

#[test]
fn resolver_reads_the_request_route() {
    let body = JsonObject::new();
    let path_params = BTreeSet::new();
    let request = RequestSnapshot {
        route: "/openai/deployments/dep-model/chat/completions",
        body: &body,
        headers: None,
        query: None,
        dispatch: DispatchContext {
            endpoint_marker: None,
            path_params: &path_params,
        },
        team_id: None,
    };
    let resolver = RequestModelResolver {
        catalog: Arc::new(EmptyCatalog),
        decoders: Arc::new(RecordingDecoders::default()),
    };
    assert_eq!(
        resolver.resolve(&request),
        Ok(RequestedModels::One("dep-model".to_owned()))
    );
}
