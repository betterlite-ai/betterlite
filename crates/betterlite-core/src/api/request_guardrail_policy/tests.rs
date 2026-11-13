#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use betterlite_policy::attachment::AttachmentRegistry;
use betterlite_policy::registry::{PolicyRegistry, RawPolicy};
use betterlite_policy::values::PolicyAttachment;
use indexmap::IndexMap;
use rstest::rstest;
use tracing::subscriber::{DefaultGuard, set_default};

use super::error::{GuardrailPolicyErrorFacts, RequestGuardrailPolicyError};
use super::{
    GuardrailMetadata, GuardrailPolicyService, GuardrailPolicySettings, GuardrailPrincipal,
    RequestGuardrailPolicyInput, RequestGuardrailPolicyResult,
};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};
use crate::api::request_destination::RequestData;

fn data(source: &str) -> RequestData {
    serde_json::from_str(source).unwrap()
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn metadata(guardrails: &[&str], policies: &[&str]) -> GuardrailMetadata {
    GuardrailMetadata {
        guardrails: names(guardrails),
        policies: names(policies),
    }
}

fn request(data: RequestData) -> RequestGuardrailPolicyInput {
    RequestGuardrailPolicyInput {
        data,
        principal: GuardrailPrincipal::default(),
        key_metadata: GuardrailMetadata::default(),
        team_metadata: GuardrailMetadata::default(),
        project_metadata: GuardrailMetadata::default(),
        metadata_variable_name: "metadata".to_owned(),
    }
}

struct World {
    registry: PolicyRegistry,
    attachments: AttachmentRegistry,
}

fn world(policies: &str, attachments: &str) -> World {
    let mut registry = PolicyRegistry::default();
    let parsed: BTreeMap<String, RawPolicy> = serde_json::from_str(policies).unwrap();
    registry.load_policies(parsed).unwrap();
    let mut attachment_registry = AttachmentRegistry::default();
    let list: Vec<PolicyAttachment> = serde_json::from_str(attachments).unwrap();
    attachment_registry.load_attachments(list);
    World {
        registry,
        attachments: attachment_registry,
    }
}

fn uninitialized_world() -> World {
    World {
        registry: PolicyRegistry::default(),
        attachments: AttachmentRegistry::default(),
    }
}

fn resolve(
    world: &World,
    input: &RequestGuardrailPolicyInput,
) -> Result<RequestGuardrailPolicyResult, RequestGuardrailPolicyError> {
    let service = GuardrailPolicyService {
        policies: &world.registry,
        attachments: &world.attachments,
        settings: GuardrailPolicySettings {
            premium_user: true,
            limits: ApiLimits::default(),
        },
    };
    service.resolve(input)
}

fn bucket(data: &RequestData) -> Option<&IndexMap<String, Json>> {
    let Some(Json::Object(metadata)) = data.get("metadata") else {
        return None;
    };
    Some(metadata)
}

fn text_list(value: Option<&Json>) -> Vec<String> {
    let Some(Json::List(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Json::Text(text) => Some(text.clone()),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

fn guardrails(result: &RequestGuardrailPolicyResult) -> Vec<String> {
    text_list(bucket(&result.data).and_then(|metadata| metadata.get("guardrails")))
}

fn applied_policies(result: &RequestGuardrailPolicyResult) -> Vec<String> {
    text_list(bucket(&result.data).and_then(|metadata| metadata.get("applied_policies")))
}

struct OrderCase {
    key: &'static [&'static str],
    team: &'static [&'static str],
    project: &'static [&'static str],
    team_alias: Option<&'static str>,
    data: &'static str,
    policies: &'static str,
    attachments: &'static str,
    expected: &'static [&'static str],
}

#[rstest]
#[case::key_then_team_then_project(OrderCase {
    key: &["key-a"],
    team: &["team-a"],
    project: &["proj-a"],
    team_alias: None,
    data: r#"{"model": "gpt-4"}"#,
    policies: "{}",
    attachments: "[]",
    expected: &["key-a", "team-a", "proj-a"],
})]
#[case::policy_guardrails_after_direct_ones(OrderCase {
    key: &["key-a"],
    team: &[],
    project: &[],
    team_alias: None,
    data: r#"{"model": "gpt-4"}"#,
    policies: r#"{"p1": {"guardrails": {"add": ["pii"]}}}"#,
    attachments: "[]",
    expected: &["key-a", "pii"],
})]
#[case::attachment_guardrails_after_metadata_ones(OrderCase {
    key: &["key-a"],
    team: &[],
    project: &[],
    team_alias: Some("team-a"),
    data: r#"{"model": "gpt-4"}"#,
    policies: r#"{"p2": {"guardrails": {"add": ["audit"]}}}"#,
    attachments: r#"[{"policy": "p2", "teams": ["team-a"]}]"#,
    expected: &["key-a", "audit"],
})]
#[case::request_body_guardrails_append_last(OrderCase {
    key: &[],
    team: &[],
    project: &[],
    team_alias: None,
    data: r#"{"model": "gpt-4", "guardrails": ["body-a", "body-b"]}"#,
    policies: r#"{"p2": {"guardrails": {"add": ["audit"]}}}"#,
    attachments: r#"[{"policy": "p2", "scope": "*"}]"#,
    expected: &["audit", "body-a", "body-b"],
})]
#[case::existing_metadata_guardrails_stay_first(OrderCase {
    key: &[],
    team: &[],
    project: &[],
    team_alias: None,
    data: r#"{"model": "gpt-4", "metadata": {"guardrails": ["existing"]}, "guardrails": ["body-a"]}"#,
    policies: "{}",
    attachments: "[]",
    expected: &["existing", "body-a"],
})]
#[case::direct_guardrails_replace_existing_metadata(OrderCase {
    key: &["key-a"],
    team: &[],
    project: &[],
    team_alias: None,
    data: r#"{"model": "gpt-4", "metadata": {"guardrails": ["existing"]}}"#,
    policies: "{}",
    attachments: "[]",
    expected: &["key-a"],
})]
fn guardrail_policy_order(#[case] case: OrderCase) {
    let world = world(case.policies, case.attachments);
    let mut input = request(data(case.data));
    input.principal.team_alias = case.team_alias.map(str::to_owned);
    input.key_metadata = metadata(case.key, &[]);
    input.team_metadata = metadata(case.team, &[]);
    input.project_metadata = metadata(case.project, &[]);
    let result = resolve(&world, &input).unwrap();
    assert_eq!(guardrails(&result), names(case.expected));
}

struct ExcludedCase {
    policies: &'static str,
    policy_names: &'static [&'static str],
    expected_guardrails: &'static [&'static str],
    expected_applied: &'static [&'static str],
    expected_excluded: &'static [&'static str],
}

#[rstest]
#[case::missing_alongside_present(ExcludedCase {
    policies: r#"{"present": {"guardrails": {"add": ["pii"]}}}"#,
    policy_names: &["missing", "present"],
    expected_guardrails: &["pii"],
    expected_applied: &["missing", "present"],
    expected_excluded: &["missing"],
})]
#[case::missing_alone(ExcludedCase {
    policies: "{}",
    policy_names: &["missing"],
    expected_guardrails: &[],
    expected_applied: &[],
    expected_excluded: &["missing"],
})]
fn excluded_policy_diagnostic(#[case] case: ExcludedCase) {
    let world = world(case.policies, "[]");
    let mut input = request(data(r#"{"model": "gpt-4"}"#));
    input.key_metadata = metadata(&[], case.policy_names);
    let (captured, guard) = capture();
    let result = resolve(&world, &input).unwrap();
    drop(guard);
    assert_eq!(guardrails(&result), names(case.expected_guardrails));
    assert_eq!(applied_policies(&result), names(case.expected_applied));
    assert_eq!(result.excluded_policies, names(case.expected_excluded));
    let logged = captured.text();
    assert!(logged.contains("WARN"));
    for policy_name in case.policy_names {
        assert!(logged.contains(policy_name));
    }
}

struct EmptyCase {
    data: &'static str,
    policies: Option<&'static str>,
    expected: &'static str,
}

#[rstest]
#[case::uninitialized_registry_leaves_data_alone(EmptyCase {
    data: r#"{"model": "gpt-4"}"#,
    policies: None,
    expected: r#"{"model": "gpt-4"}"#,
})]
#[case::initialized_registry_creates_an_empty_bucket(EmptyCase {
    data: r#"{"model": "gpt-4"}"#,
    policies: Some("{}"),
    expected: r#"{"model": "gpt-4", "metadata": {}}"#,
})]
#[case::request_policies_are_removed(EmptyCase {
    data: r#"{"model": "gpt-4", "policies": []}"#,
    policies: None,
    expected: r#"{"model": "gpt-4"}"#,
})]
#[case::empty_request_guardrails_are_moved(EmptyCase {
    data: r#"{"model": "gpt-4", "metadata": {}, "guardrails": []}"#,
    policies: None,
    expected: r#"{"model": "gpt-4", "metadata": {"guardrails": []}}"#,
})]
fn empty_policy_selection(#[case] case: EmptyCase) {
    let world = case
        .policies
        .map_or_else(uninitialized_world, |policies| world(policies, "[]"));
    let input = request(data(case.data));
    let result = resolve(&world, &input).unwrap();
    assert_eq!(result.data, data(case.expected));
    assert!(result.excluded_policies.is_empty());
}

const NOT_PREMIUM: GuardrailPolicyErrorFacts = GuardrailPolicyErrorFacts {
    status: 403,
    error_type: "permission_error",
    param: None,
    code: "403",
};

#[rstest]
#[case::guardrails_from_key(
    GuardrailMetadata { guardrails: names(&["key-a"]), policies: Vec::new() },
    Some(NOT_PREMIUM),
)]
#[case::policies_from_key(
    GuardrailMetadata { guardrails: Vec::new(), policies: names(&["p1"]) },
    Some(NOT_PREMIUM),
)]
#[case::nothing_configured(GuardrailMetadata::default(), None)]
fn not_premium_metadata_guardrails(
    #[case] key_metadata: GuardrailMetadata,
    #[case] expected: Option<GuardrailPolicyErrorFacts>,
) {
    let world = world(r#"{"p1": {"guardrails": {"add": ["pii"]}}}"#, "[]");
    let mut input = request(data(r#"{"model": "gpt-4"}"#));
    input.key_metadata = key_metadata;
    let service = GuardrailPolicyService {
        policies: &world.registry,
        attachments: &world.attachments,
        settings: GuardrailPolicySettings {
            premium_user: false,
            limits: ApiLimits::default(),
        },
    };
    match expected {
        None => {
            let result = service.resolve(&input).unwrap();
            assert!(guardrails(&result).is_empty());
        }
        Some(facts) => {
            let error = service.resolve(&input).unwrap_err();
            assert!(matches!(error, RequestGuardrailPolicyError::NotPremium));
            assert_eq!(error.facts(), facts);
        }
    }
}

#[test]
fn policy_version_id_selection() {
    let mut world = world(r#"{"published": {"guardrails": {"add": ["pii"]}}}"#, "[]");
    let policy = world.registry.policies["published"].clone();
    world
        .registry
        .policies_by_id
        .insert("abc".to_owned(), ("published".to_owned(), policy));
    let input = request(data(
        r#"{"model": "gpt-4", "policies": ["policy_abc", "policy_", "policy_gone"]}"#,
    ));
    let result = resolve(&world, &input).unwrap();
    assert_eq!(guardrails(&result), names(&["pii"]));
    assert_eq!(applied_policies(&result), names(&["published"]));
}

#[test]
fn pipeline_metadata_is_written() {
    let world = world(
        r#"{"piped": {"pipeline": {"mode": "post_call", "steps": [{"guardrail": "g1"}, {"guardrail": "g2", "on_fail": "next"}]}}}"#,
        r#"[{"policy": "piped", "scope": "*"}]"#,
    );
    let input = request(data(r#"{"model": "gpt-4"}"#));
    let result = resolve(&world, &input).unwrap();
    let metadata = bucket(&result.data).unwrap();
    assert_eq!(
        text_list(metadata.get("_pipeline_managed_guardrails")),
        names(&["g1", "g2"])
    );
    let Some(Json::List(pipelines)) = metadata.get("_guardrail_pipelines") else {
        panic!("pipelines are a list");
    };
    assert_eq!(pipelines.len(), 1);
    let Some(Json::List(pair)) = pipelines.first() else {
        panic!("a pipeline is a [name, definition] pair");
    };
    assert_eq!(pair.first(), Some(&Json::Text("piped".to_owned())));
    let Some(Json::Object(definition)) = pair.get(1) else {
        panic!("the pipeline definition is an object");
    };
    assert_eq!(
        definition.get("mode"),
        Some(&Json::Text("post_call".to_owned()))
    );
    assert_eq!(guardrails(&result), Vec::<String>::new());
    assert_eq!(applied_policies(&result), names(&["piped"]));
}

#[test]
fn request_policy_limit_is_enforced() {
    let world = world("{}", "[]");
    let input = request(data(r#"{"model": "gpt-4", "policies": ["a", "b"]}"#));
    let defaults = ApiLimits::default();
    let service = GuardrailPolicyService {
        policies: &world.registry,
        attachments: &world.attachments,
        settings: GuardrailPolicySettings {
            premium_user: true,
            limits: ApiLimits {
                max_json_bytes: defaults.max_json_bytes,
                max_event_bytes: defaults.max_event_bytes,
                max_items: 1,
            },
        },
    };
    let error = service.resolve(&input).unwrap_err();
    assert!(matches!(
        error,
        RequestGuardrailPolicyError::Limit(ApiLimitError::TooManyItems {
            observed: 2,
            limit: 1
        })
    ));
    assert_eq!(error.facts().code, "too_many_items");
}

#[rstest]
#[case::policies_not_a_list(r#"{"model": "gpt-4", "policies": "p1"}"#, "policies")]
#[case::guardrails_not_a_list(r#"{"model": "gpt-4", "guardrails": 3}"#, "guardrails")]
#[case::include_flag_not_a_bool(
    r#"{"model": "gpt-4", "include_guardrail_response": 1}"#,
    "include_guardrail_response"
)]
#[case::guardrail_config_not_an_object(
    r#"{"model": "gpt-4", "guardrail_config": 3}"#,
    "guardrail_config"
)]
#[case::model_not_text(r#"{"model": 3}"#, "model")]
#[case::existing_guardrails_not_a_list(
    r#"{"model": "gpt-4", "metadata": {"guardrails": 5}}"#,
    "guardrails"
)]
fn untyped_fields_are_named(#[case] source: &'static str, #[case] expected_field: &'static str) {
    let world = world(r#"{"p1": {"guardrails": {"add": ["pii"]}}}"#, "[]");
    let mut input = request(data(source));
    input.key_metadata = metadata(&[], &["p1"]);
    let error = resolve(&world, &input).unwrap_err();
    let RequestGuardrailPolicyError::NotTyped { field, .. } = error else {
        panic!("a field type error");
    };
    assert_eq!(field, expected_field);
}

#[test]
fn metadata_policies_are_resolved_once() {
    let world = world(
        r#"{"p1": {"guardrails": {"add": ["pii"]}}}"#,
        r#"[{"policy": "p1", "scope": "*"}]"#,
    );
    let mut input = request(data(r#"{"model": "gpt-4"}"#));
    input.key_metadata = metadata(&[], &["p1"]);
    input.team_metadata = metadata(&[], &["p1"]);
    let result = resolve(&world, &input).unwrap();
    assert_eq!(guardrails(&result), names(&["pii"]));
    assert_eq!(applied_policies(&result), names(&["p1"]));
}

#[test]
fn policy_sources_record_the_attachment_reason() {
    let world = world(
        r#"{"p1": {"guardrails": {"add": ["pii"]}}}"#,
        r#"[{"policy": "p1", "keys": ["key-a"]}]"#,
    );
    let mut input = request(data(r#"{"model": "gpt-4"}"#));
    input.principal.key_alias = Some("key-a".to_owned());
    let result = resolve(&world, &input).unwrap();
    let metadata = bucket(&result.data).unwrap();
    let Some(Json::Object(sources)) = metadata.get("policy_sources") else {
        panic!("policy sources are an object");
    };
    assert_eq!(sources.get("p1"), Some(&Json::Text("key:key-a".to_owned())));
}

#[test]
fn a_policy_that_does_not_apply_contributes_nothing() {
    let world = world(
        r#"{"piped": {"condition": {"model": "other"}, "guardrails": {"add": ["pii"]}, "pipeline": {"mode": "pre_call", "steps": [{"guardrail": "g1"}]}}}"#,
        r#"[{"policy": "piped", "scope": "*"}]"#,
    );
    let input = request(data(r#"{"model": "gpt-4"}"#));
    let result = resolve(&world, &input).unwrap();
    assert!(guardrails(&result).is_empty());
    assert!(applied_policies(&result).is_empty());
    assert!(
        bucket(&result.data)
            .unwrap()
            .get("_guardrail_pipelines")
            .is_none()
    );
}

#[test]
fn tags_from_metadata_scope_a_policy() {
    let world = world(
        r#"{"tagged": {"guardrails": {"add": ["pii"]}}}"#,
        r#"[{"policy": "tagged", "tags": ["release"]}]"#,
    );
    let input = request(data(
        r#"{"model": "gpt-4", "metadata": {"tags": ["release"]}}"#,
    ));
    let result = resolve(&world, &input).unwrap();
    assert_eq!(guardrails(&result), names(&["pii"]));
}

#[test]
fn resolution_leaves_the_callers_data_untouched() {
    let world = world(r#"{"p1": {"guardrails": {"add": ["pii"]}}}"#, "[]");
    let mut input = request(data(r#"{"model": "gpt-4", "guardrails": ["body-a"]}"#));
    input.key_metadata = metadata(&[], &["p1"]);
    let original = input.data.clone();
    let result = resolve(&world, &input).unwrap();
    assert_eq!(input.data, original);
    assert_ne!(result.data, original);
}

#[test]
fn inherited_guardrails_drop_what_the_child_removes() {
    let world = world(
        r#"{"base": {"guardrails": {"add": ["pii", "audit"]}}, "child": {"inherit": "base", "guardrails": {"remove": ["pii"]}}}"#,
        "[]",
    );
    let mut input = request(data(r#"{"model": "gpt-4"}"#));
    input.key_metadata = metadata(&[], &["child"]);
    let result = resolve(&world, &input).unwrap();
    assert_eq!(guardrails(&result), names(&["audit"]));
}

#[test]
fn direct_guardrails_survive_an_uninitialized_registry() {
    let world = uninitialized_world();
    let mut input = request(data(r#"{"model": "gpt-4"}"#));
    input.key_metadata = metadata(&["key-a"], &[]);
    input.team_metadata = metadata(&[], &["p1"]);
    let result = resolve(&world, &input).unwrap();
    assert_eq!(guardrails(&result), names(&["key-a"]));
    assert!(result.excluded_policies.is_empty());
}

#[test]
fn a_null_include_flag_becomes_false() {
    let world = world("{}", "[]");
    let input = request(data(
        r#"{"model": "gpt-4", "metadata": {}, "include_guardrail_response": null}"#,
    ));
    let result = resolve(&world, &input).unwrap();
    let metadata = bucket(&result.data).unwrap();
    assert_eq!(
        metadata.get("include_guardrail_response"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn guardrail_config_is_merged_into_metadata() {
    let world = world("{}", "[]");
    let input = request(data(
        r#"{"model": "gpt-4", "metadata": {"guardrail_config": {"a": 1, "b": 2}}, "guardrail_config": {"b": 3}}"#,
    ));
    let result = resolve(&world, &input).unwrap();
    let metadata = bucket(&result.data).unwrap();
    let Some(Json::Object(config)) = metadata.get("guardrail_config") else {
        panic!("guardrail config is an object");
    };
    assert_eq!(
        config.get("b"),
        Some(&Json::Number(serde_json::Number::from(3_u8)))
    );
}

#[test]
fn metadata_policies_use_the_request_model() {
    let world = world(
        r#"{"conditional": {"condition": {"model": "gpt-4"}, "guardrails": {"add": ["pii"]}}}"#,
        "[]",
    );
    let mut input = request(data(r#"{"model": "gpt-4"}"#));
    input.key_metadata = metadata(&[], &["conditional"]);
    let result = resolve(&world, &input).unwrap();
    assert_eq!(guardrails(&result), names(&["pii"]));
}

#[test]
fn metadata_policies_skip_a_model_that_does_not_match() {
    let world = world(
        r#"{"conditional": {"condition": {"model": "other"}, "guardrails": {"add": ["pii"]}}}"#,
        "[]",
    );
    let mut input = request(data(r#"{"model": "gpt-4"}"#));
    input.key_metadata = metadata(&[], &["conditional"]);
    let result = resolve(&world, &input).unwrap();
    assert!(guardrails(&result).is_empty());
    assert!(applied_policies(&result).is_empty());
}

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|error| io::Error::other(error.to_string()))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();
    (captured, set_default(subscriber))
}
