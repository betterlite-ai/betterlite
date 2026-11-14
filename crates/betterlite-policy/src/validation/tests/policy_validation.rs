#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;

use super::support::{
    AliasKind, AliasOutcome, FakeFailure, FakeRegistry, Warnings, check_alias_case, database,
    pipeline_policy, policies, policy, registry, router, texts,
};
use crate::pipeline::json_value::JsonValue;
use crate::validation::PolicyValidator;
use crate::validation::inheritance::{MAX_INHERITANCE_DEPTH, validate_inheritance_chain};
use crate::validation::lookup::{NoDatabase, NoRouter};
use crate::validation::types::{
    PolicyValidationError, PolicyValidationErrorType, PolicyValidationResponse,
};
use crate::values::Policy;

type Offline = PolicyValidator<FakeRegistry, NoDatabase, NoRouter>;

fn offline(names: &[&str]) -> Offline {
    PolicyValidator {
        guardrails: registry(names),
        database: None,
        router: None,
    }
}

fn expected(
    policy_name: &str,
    error_type: PolicyValidationErrorType,
    message: &str,
    field: Option<&str>,
    value: Option<&str>,
) -> PolicyValidationError {
    PolicyValidationError {
        policy_name: policy_name.to_owned(),
        error_type,
        message: message.to_owned(),
        field: field.map(str::to_owned),
        value: value.map(str::to_owned),
    }
}

fn config(text: &str) -> BTreeMap<String, JsonValue> {
    serde_json::from_str(text).unwrap()
}

#[rstest]
#[case::invalid_guardrail(PolicyValidationErrorType::InvalidGuardrail, "invalid_guardrail")]
#[case::invalid_team(PolicyValidationErrorType::InvalidTeam, "invalid_team")]
#[case::invalid_key(PolicyValidationErrorType::InvalidKey, "invalid_key")]
#[case::invalid_model(PolicyValidationErrorType::InvalidModel, "invalid_model")]
#[case::invalid_inheritance(PolicyValidationErrorType::InvalidInheritance, "invalid_inheritance")]
#[case::circular_inheritance(
    PolicyValidationErrorType::CircularInheritance,
    "circular_inheritance"
)]
#[case::invalid_scope(PolicyValidationErrorType::InvalidScope, "invalid_scope")]
#[case::invalid_syntax(PolicyValidationErrorType::InvalidSyntax, "invalid_syntax")]
fn error_types_use_the_wire_value(
    #[case] error_type: PolicyValidationErrorType,
    #[case] wire: &str,
) {
    let text = serde_json::to_string(&error_type).unwrap();
    assert_eq!(text, format!("\"{wire}\""));
    let parsed: PolicyValidationErrorType = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, error_type);
}

#[test]
fn error_types_reject_an_unknown_value() {
    let parsed = serde_json::from_str::<PolicyValidationErrorType>("\"INVALID_TEAM\"");
    parsed.unwrap_err();
}

#[test]
fn error_record_serializes_every_field_with_nulls() {
    let record = expected(
        "p",
        PolicyValidationErrorType::InvalidSyntax,
        "bad",
        None,
        None,
    );
    let text = serde_json::to_string(&record).unwrap();
    assert_eq!(
        text,
        r#"{"policy_name":"p","error_type":"invalid_syntax","message":"bad","field":null,"value":null}"#
    );
}

#[test]
fn error_record_round_trips_field_and_value() {
    let record = expected(
        "p",
        PolicyValidationErrorType::InvalidTeam,
        "gone",
        Some("teams"),
        Some("alpha"),
    );
    let parsed: PolicyValidationError =
        serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
    assert_eq!(parsed, record);
}

#[test]
fn error_record_defaults_field_and_value_to_none() {
    let text = r#"{"policy_name":"p","error_type":"invalid_key","message":"m"}"#;
    let parsed: PolicyValidationError = serde_json::from_str(text).unwrap();
    assert_eq!(parsed.field, None);
    assert_eq!(parsed.value, None);
}

#[rstest]
#[case::extra_field(r#"{"policy_name":"p","error_type":"invalid_key","message":"m","extra":1}"#)]
#[case::missing_policy_name(r#"{"error_type":"invalid_key","message":"m"}"#)]
#[case::missing_error_type(r#"{"policy_name":"p","message":"m"}"#)]
#[case::missing_message(r#"{"policy_name":"p","error_type":"invalid_key"}"#)]
fn error_record_rejects_bad_input(#[case] text: &str) {
    serde_json::from_str::<PolicyValidationError>(text).unwrap_err();
}

#[test]
fn response_records_serialize_valid_errors_and_warnings() {
    let warning = expected("p", PolicyValidationErrorType::InvalidKey, "w", None, None);
    let response = PolicyValidationResponse {
        valid: true,
        errors: Vec::new(),
        warnings: vec![warning],
    };
    let text = serde_json::to_string(&response).unwrap();
    assert_eq!(
        text,
        r#"{"valid":true,"errors":[],"warnings":[{"policy_name":"p","error_type":"invalid_key","message":"w","field":null,"value":null}]}"#
    );
}

#[test]
fn response_records_default_missing_lists_to_empty() {
    let parsed: PolicyValidationResponse = serde_json::from_str(r#"{"valid":false}"#).unwrap();
    assert!(!parsed.valid);
    assert!(parsed.errors.is_empty());
    assert!(parsed.warnings.is_empty());
}

#[rstest]
#[case::missing_valid(r#"{"errors":[],"warnings":[]}"#)]
#[case::extra_field(r#"{"valid":true,"extra":1}"#)]
fn response_records_reject_bad_input(#[case] text: &str) {
    serde_json::from_str::<PolicyValidationResponse>(text).unwrap_err();
}

#[tokio::test]
async fn validator_init_without_dependencies_assumes_everything_exists() {
    let validator = offline(&[]);
    assert!(validator.database.is_none());
    assert!(validator.router.is_none());
    assert!(validator.check_team_alias_exists("any").await);
    assert!(validator.check_key_alias_exists("any").await);
    assert!(validator.check_model_exists("any"));
}

#[tokio::test]
async fn validator_init_keeps_the_given_database_and_router() {
    let validator = PolicyValidator {
        guardrails: registry(&[]),
        database: Some(database(&["red"], &[])),
        router: Some(router(&["gpt"], &[])),
    };
    assert!(validator.check_team_alias_exists("red").await);
    assert!(!validator.check_team_alias_exists("blue").await);
    assert!(validator.check_model_exists("gpt"));
    assert!(!validator.check_model_exists("other"));
}

#[rstest]
#[case::names(&["b", "a"], &["a", "b"])]
#[case::empty_names_are_dropped(&["", "a"], &["a"])]
#[case::duplicates_collapse(&["a", "a"], &["a"])]
#[case::none(&[], &[])]
fn guardrail_lookup_returns_the_registered_names(
    #[case] registered: &[&str],
    #[case] available: &[&str],
) {
    let warnings = Warnings::capture();
    let found = offline(registered).get_available_guardrails();
    let want: Vec<String> = texts(available);
    assert_eq!(found.into_iter().collect::<Vec<_>>(), want);
    assert!(warnings.messages().is_empty());
}

#[test]
fn guardrail_lookup_failure_warns_and_returns_the_empty_set() {
    let warnings = Warnings::capture();
    let validator: Offline = PolicyValidator {
        guardrails: FakeRegistry {
            names: None,
            calls: AtomicUsize::new(0),
        },
        database: None,
        router: None,
    };
    assert!(validator.get_available_guardrails().is_empty());
    assert_eq!(
        warnings.messages(),
        vec![format!(
            "Could not get guardrails from registry: {FakeFailure}"
        )]
    );
}

#[test]
fn empty_registry_skips_guardrail_validation_for_add_and_remove() {
    let all = policies(vec![("p", policy(None, &["missing"], &["gone"]))]);
    let response = offline(&[]).validate_policies(&all, true);
    assert_eq!(
        response,
        PolicyValidationResponse {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new()
        }
    );
}

#[test]
fn empty_registry_skips_the_pipeline_registry_check() {
    let all = policies(vec![("p", pipeline_policy(&["a"], &["a"]))]);
    assert!(offline(&[]).validate_policies(&all, true).valid);
}

#[rstest]
#[case::exists(&["red"], AliasOutcome::Found)]
#[case::missing(&["blue"], AliasOutcome::Missing)]
#[case::database_failure(&[], AliasOutcome::Failure)]
#[tokio::test]
async fn team_alias_checks_the_database(#[case] known: &[&str], #[case] outcome: AliasOutcome) {
    check_alias_case(AliasKind::Team, "red", known, outcome).await;
}

#[rstest]
#[case::exists(&["svc"], AliasOutcome::Found)]
#[case::missing(&["other"], AliasOutcome::Missing)]
#[case::database_failure(&[], AliasOutcome::Failure)]
#[tokio::test]
async fn key_alias_checks_the_database(#[case] known: &[&str], #[case] outcome: AliasOutcome) {
    check_alias_case(AliasKind::Key, "svc", known, outcome).await;
}

#[derive(Clone, Copy, Debug)]
enum RouterOutcome {
    Answers,
    FailsNames,
    FailsPatterns,
}

#[rstest]
#[case::router_model_name(&["gpt"], &[], RouterOutcome::Answers, true, 0)]
#[case::pattern_match(&[], &["gpt"], RouterOutcome::Answers, true, 1)]
#[case::unknown(&["other"], &["other"], RouterOutcome::Answers, false, 1)]
#[case::name_lookup_failure(&["gpt"], &["gpt"], RouterOutcome::FailsNames, true, 0)]
#[case::pattern_lookup_failure(&[], &["gpt"], RouterOutcome::FailsPatterns, true, 1)]
fn model_exists_checks_names_then_patterns(
    #[case] model_names: &[&str],
    #[case] patterns: &[&str],
    #[case] outcome: RouterOutcome,
    #[case] want: bool,
    #[case] pattern_calls: usize,
) {
    let warnings = Warnings::capture();
    let mut fake = router(model_names, patterns);
    fake.failing_names = matches!(outcome, RouterOutcome::FailsNames);
    fake.failing_patterns = matches!(outcome, RouterOutcome::FailsPatterns);
    let validator = PolicyValidator {
        guardrails: registry(&[]),
        database: None::<NoDatabase>,
        router: Some(fake),
    };
    assert_eq!(validator.check_model_exists("gpt"), want);
    let stored = validator.router.as_ref().unwrap();
    assert_eq!(stored.pattern_calls.load(Ordering::SeqCst), pattern_calls);
    let logged = warnings.messages();
    if matches!(outcome, RouterOutcome::Answers) {
        assert!(logged.is_empty());
    } else {
        assert_eq!(
            logged,
            vec![format!("Could not check model 'gpt': {FakeFailure}")]
        );
    }
}

#[tokio::test]
async fn scope_validation_reports_concrete_missing_entries_in_order() {
    let validator = PolicyValidator {
        guardrails: registry(&[]),
        database: Some(database(&["red"], &["svc"])),
        router: Some(router(&["gpt"], &[])),
    };
    let errors = validator
        .find_invalid_scope_entries(
            "p",
            &texts(&["red", "blue"]),
            &texts(&["svc", "gone"]),
            &texts(&["gpt", "nope"]),
        )
        .await;
    assert_eq!(
        errors,
        vec![
            expected(
                "p",
                PolicyValidationErrorType::InvalidTeam,
                "Team 'blue' does not exist. Reference an existing team or use a wildcard pattern (e.g. 'blue*') to match by prefix.",
                Some("teams"),
                Some("blue"),
            ),
            expected(
                "p",
                PolicyValidationErrorType::InvalidKey,
                "Key 'gone' does not exist. Reference an existing key or use a wildcard pattern (e.g. 'gone*') to match by prefix.",
                Some("keys"),
                Some("gone"),
            ),
            expected(
                "p",
                PolicyValidationErrorType::InvalidModel,
                "Model 'nope' does not exist. Reference an existing model or use a wildcard pattern (e.g. 'nope*') to match by prefix.",
                Some("models"),
                Some("nope"),
            ),
        ]
    );
}

#[tokio::test]
async fn scope_validation_without_entries_or_dependencies_finds_nothing() {
    let validator = offline(&[]);
    let errors = validator
        .find_invalid_scope_entries("p", &texts(&["a"]), &texts(&["b"]), &texts(&["c"]))
        .await;
    assert!(errors.is_empty());
    assert!(
        validator
            .find_invalid_scope_entries("p", &[], &[], &[])
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn scope_validation_never_queries_wildcard_entries() {
    let validator = PolicyValidator {
        guardrails: registry(&[]),
        database: Some(database(&[], &[])),
        router: Some(router(&[], &[])),
    };
    let errors = validator
        .find_invalid_scope_entries(
            "p",
            &texts(&["team-*", "*"]),
            &texts(&["key-*"]),
            &texts(&["gpt-*"]),
        )
        .await;
    assert!(errors.is_empty());
    assert!(validator.database.as_ref().unwrap().recorded().is_empty());
    let pattern_calls = validator
        .router
        .as_ref()
        .unwrap()
        .pattern_calls
        .load(Ordering::SeqCst);
    assert_eq!(pattern_calls, 0);
}

#[tokio::test]
async fn scope_validation_does_not_report_the_lookup_failures() {
    let mut failing = database(&[], &[]);
    failing.failing = true;
    let validator = PolicyValidator {
        guardrails: registry(&[]),
        database: Some(failing),
        router: None::<NoRouter>,
    };
    let errors = validator
        .find_invalid_scope_entries("p", &texts(&["a"]), &texts(&["b"]), &[])
        .await;
    assert!(errors.is_empty());
}

#[rstest]
#[case::trailing_star("team-*", false)]
#[case::bare_star("*", false)]
#[case::concrete("team-prod", true)]
#[case::leading_star("*team", true)]
#[case::inner_star("te*am", true)]
#[case::question_mark("team?", true)]
#[case::empty("", true)]
#[tokio::test]
async fn wildcard_detection_uses_only_the_trailing_star(
    #[case] entry: &str,
    #[case] validated: bool,
) {
    let validator = PolicyValidator {
        guardrails: registry(&[]),
        database: Some(database(&[], &[])),
        router: None::<NoRouter>,
    };
    let errors = validator
        .find_invalid_scope_entries("p", &texts(&[entry]), &[], &[])
        .await;
    assert_eq!(errors.len(), usize::from(validated));
    assert_eq!(
        validator.database.as_ref().unwrap().recorded().len(),
        usize::from(validated)
    );
}

#[rstest]
#[case::no_parent(&[("a", None)], "a")]
#[case::parent_chain(&[("a", Some("b")), ("b", Some("c")), ("c", None)], "a")]
#[case::empty_inherit_is_none(&[("a", Some(""))], "a")]
#[case::unknown_policy(&[("a", None)], "missing")]
fn inheritance_valid_chains_report_nothing(
    #[case] chain: &[(&str, Option<&str>)],
    #[case] start: &str,
) {
    let all = policies(
        chain
            .iter()
            .map(|(name, parent)| (*name, policy(*parent, &[], &[])))
            .collect(),
    );
    assert_eq!(validate_inheritance_chain(start, &all), None);
}

#[test]
fn inheritance_missing_parent_is_a_blocking_error() {
    let all = policies(vec![("a", policy(Some("ghost"), &[], &[]))]);
    assert_eq!(
        validate_inheritance_chain("a", &all),
        Some(expected(
            "a",
            PolicyValidationErrorType::InvalidInheritance,
            "Parent policy 'ghost' not found",
            Some("inherit"),
            Some("ghost"),
        ))
    );
}

#[test]
fn inheritance_missing_parent_deeper_in_the_chain_names_the_child() {
    let all = policies(vec![
        ("a", policy(Some("b"), &[], &[])),
        ("b", policy(Some("ghost"), &[], &[])),
    ]);
    let error = validate_inheritance_chain("a", &all).unwrap();
    assert_eq!(error.policy_name, "b");
    assert_eq!(
        error.error_type,
        PolicyValidationErrorType::InvalidInheritance
    );
}

#[rstest]
#[case::self_cycle(&[("a", "a")], "a", "a", "Circular inheritance detected: a -> a")]
#[case::two_cycle(&[("a", "b"), ("b", "a")], "a", "a", "Circular inheritance detected: a -> b -> a")]
#[case::tail_into_cycle(
    &[("t", "a"), ("a", "b"), ("b", "a")],
    "t",
    "a",
    "Circular inheritance detected: t -> a -> b -> a"
)]
fn inheritance_cycles_name_the_repeated_policy(
    #[case] links: &[(&str, &str)],
    #[case] start: &str,
    #[case] repeated: &str,
    #[case] message: &str,
) {
    let all = policies(
        links
            .iter()
            .map(|(name, parent)| (*name, policy(Some(*parent), &[], &[])))
            .collect(),
    );
    assert_eq!(
        validate_inheritance_chain(start, &all),
        Some(expected(
            repeated,
            PolicyValidationErrorType::CircularInheritance,
            message,
            Some("inherit"),
            None,
        ))
    );
}

fn linear_chain(length: usize, last_parent: Option<&str>) -> BTreeMap<String, Policy> {
    let mut chain = BTreeMap::new();
    for index in 0..length {
        let next = index.saturating_add(1);
        let parent = if next < length {
            Some(format!("p{next}"))
        } else {
            last_parent.map(str::to_owned)
        };
        chain.insert(format!("p{index}"), policy(parent.as_deref(), &[], &[]));
    }
    chain
}

#[rstest]
#[case::at_the_limit(MAX_INHERITANCE_DEPTH, None)]
#[case::one_past_the_limit(MAX_INHERITANCE_DEPTH + 1, Some("p100"))]
fn inheritance_depth_limit_is_one_hundred_policies(
    #[case] length: usize,
    #[case] failing_policy: Option<&str>,
) {
    assert_eq!(MAX_INHERITANCE_DEPTH, 100);
    let error = validate_inheritance_chain("p0", &linear_chain(length, None));
    assert_eq!(
        error.as_ref().map(|found| found.policy_name.as_str()),
        failing_policy
    );
    if let Some(found) = error {
        assert_eq!(
            found.error_type,
            PolicyValidationErrorType::CircularInheritance
        );
        assert_eq!(
            found.message,
            "Inheritance chain too deep (exceeded max depth of 100)"
        );
        assert_eq!(found.field.as_deref(), Some("inherit"));
        assert_eq!(found.value, None);
    }
}

#[test]
fn inheritance_depth_error_comes_before_the_missing_parent_error() {
    let all = linear_chain(MAX_INHERITANCE_DEPTH + 1, Some("ghost"));
    let error = validate_inheritance_chain("p0", &all).unwrap();
    assert_eq!(
        error.error_type,
        PolicyValidationErrorType::CircularInheritance
    );
}

#[test]
fn inheritance_missing_parent_at_the_last_allowed_depth_is_reported() {
    let all = linear_chain(MAX_INHERITANCE_DEPTH, Some("ghost"));
    let error = validate_inheritance_chain("p0", &all).unwrap();
    assert_eq!(
        error.error_type,
        PolicyValidationErrorType::InvalidInheritance
    );
    assert_eq!(error.policy_name, "p99");
}

#[test]
fn policy_guardrails_missing_from_a_non_empty_registry_are_errors() {
    let all = policies(vec![("p", policy(None, &["known", "missing"], &[]))]);
    let response = offline(&["known"]).validate_policies(&all, true);
    assert!(!response.valid);
    assert!(response.warnings.is_empty());
    assert_eq!(
        response.errors,
        vec![expected(
            "p",
            PolicyValidationErrorType::InvalidGuardrail,
            "Guardrail 'missing' not found in guardrail registry",
            Some("guardrails.add"),
            Some("missing"),
        )]
    );
}

#[test]
fn policy_guardrails_all_registered_is_valid() {
    let all = policies(vec![("p", policy(None, &["a", "b"], &[]))]);
    let response = offline(&["a", "b", "c"]).validate_policies(&all, true);
    assert_eq!(
        response,
        PolicyValidationResponse {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new()
        }
    );
}

#[test]
fn policy_guardrails_errors_follow_policy_name_order() {
    let all = policies(vec![
        ("zeta", policy(None, &["x"], &[])),
        ("alpha", policy(None, &["y"], &[])),
    ]);
    let response = offline(&["known"]).validate_policies(&all, true);
    let names: Vec<&str> = response
        .errors
        .iter()
        .map(|found| found.policy_name.as_str())
        .collect();
    assert_eq!(names, vec!["alpha", "zeta"]);
}

#[test]
fn guardrail_remove_list_entries_missing_from_the_registry_are_warnings() {
    let all = policies(vec![("p", policy(None, &[], &["gone", "known"]))]);
    let response = offline(&["known"]).validate_policies(&all, true);
    assert!(response.valid);
    assert!(response.errors.is_empty());
    assert_eq!(
        response.warnings,
        vec![expected(
            "p",
            PolicyValidationErrorType::InvalidGuardrail,
            "Guardrail 'gone' in remove list not found in guardrail registry",
            Some("guardrails.remove"),
            Some("gone"),
        )]
    );
}

#[test]
fn guardrail_warning_is_reported_next_to_an_add_error() {
    let all = policies(vec![("p", policy(None, &["bad"], &["gone"]))]);
    let response = offline(&["known"]).validate_policies(&all, true);
    assert!(!response.valid);
    assert_eq!(response.errors.len(), 1);
    assert_eq!(response.warnings.len(), 1);
}

#[test]
fn validate_policies_reads_the_registry_once_for_every_policy() {
    let all = policies(vec![
        ("a", policy(None, &["x"], &["y"])),
        ("b", policy(None, &["x"], &["y"])),
        ("c", policy(None, &["x"], &["y"])),
    ]);
    let validator = offline(&["known"]);
    let response = validator.validate_policies(&all, true);
    assert_eq!(validator.guardrails.calls.load(Ordering::SeqCst), 1);
    assert_eq!(response.errors.len(), 3);
    assert_eq!(response.warnings.len(), 3);
}

#[rstest]
#[case::flag_on(true)]
#[case::flag_off(false)]
fn validate_policies_never_reads_the_database_flag(#[case] validate_db: bool) {
    let all = policies(vec![("p", policy(None, &["bad"], &["gone"]))]);
    let response = offline(&["known"]).validate_policies(&all, validate_db);
    assert_eq!(response.errors.len(), 1);
    assert_eq!(response.warnings.len(), 1);
}

#[test]
fn validate_policies_does_not_check_scope_entries() {
    let validator = PolicyValidator {
        guardrails: registry(&["known"]),
        database: Some(database(&[], &[])),
        router: Some(router(&[], &[])),
    };
    let all = policies(vec![("p", policy(None, &["known"], &[]))]);
    assert!(validator.validate_policies(&all, true).valid);
    assert!(validator.database.as_ref().unwrap().recorded().is_empty());
    let pattern_calls = validator
        .router
        .as_ref()
        .unwrap()
        .pattern_calls
        .load(Ordering::SeqCst);
    assert_eq!(pattern_calls, 0);
}

#[test]
fn validate_policies_orders_guardrail_pipeline_and_inheritance_errors_per_policy() {
    let mut broken = pipeline_policy(&["bad"], &["other"]);
    broken.inherit = Some("ghost".to_owned());
    let all = policies(vec![("p", broken)]);
    let response = offline(&["known"]).validate_policies(&all, true);
    let kinds: Vec<(PolicyValidationErrorType, Option<&str>)> = response
        .errors
        .iter()
        .map(|found| (found.error_type, found.field.as_deref()))
        .collect();
    assert_eq!(
        kinds,
        vec![
            (
                PolicyValidationErrorType::InvalidGuardrail,
                Some("guardrails.add")
            ),
            (
                PolicyValidationErrorType::InvalidGuardrail,
                Some("pipeline.steps")
            ),
            (
                PolicyValidationErrorType::InvalidGuardrail,
                Some("pipeline.steps")
            ),
            (
                PolicyValidationErrorType::InvalidInheritance,
                Some("inherit")
            ),
        ]
    );
}

#[test]
fn validate_policies_reports_a_cycle_once_per_policy_in_it() {
    let all = policies(vec![
        ("a", policy(Some("b"), &[], &[])),
        ("b", policy(Some("a"), &[], &[])),
    ]);
    let response = offline(&[]).validate_policies(&all, true);
    let messages: Vec<&str> = response
        .errors
        .iter()
        .map(|found| found.message.as_str())
        .collect();
    assert_eq!(
        messages,
        vec![
            "Circular inheritance detected: a -> b -> a",
            "Circular inheritance detected: b -> a -> b",
        ]
    );
}

#[test]
fn pipeline_step_outside_guardrails_add_is_an_error() {
    let all = policies(vec![("p", pipeline_policy(&["a"], &["a", "b"]))]);
    let response = offline(&[]).validate_policies(&all, true);
    assert!(!response.valid);
    assert_eq!(
        response.errors,
        vec![expected(
            "p",
            PolicyValidationErrorType::InvalidGuardrail,
            "Pipeline step 1 guardrail 'b' is not in the policy's guardrails.add list",
            Some("pipeline.steps"),
            Some("b"),
        )]
    );
}

#[test]
fn pipeline_step_missing_from_a_non_empty_registry_is_an_error() {
    let all = policies(vec![("p", pipeline_policy(&["a", "b"], &["a", "b"]))]);
    let response = offline(&["a"]).validate_policies(&all, true);
    let pipeline_errors: Vec<&PolicyValidationError> = response
        .errors
        .iter()
        .filter(|found| found.field.as_deref() == Some("pipeline.steps"))
        .collect();
    assert_eq!(
        pipeline_errors,
        vec![&expected(
            "p",
            PolicyValidationErrorType::InvalidGuardrail,
            "Pipeline step 1 guardrail 'b' not found in guardrail registry",
            Some("pipeline.steps"),
            Some("b"),
        )]
    );
}

#[test]
fn pipeline_step_failing_both_checks_reports_two_errors_membership_first() {
    let all = policies(vec![("p", pipeline_policy(&[], &["z"]))]);
    let response = offline(&["a"]).validate_policies(&all, true);
    let messages: Vec<&str> = response
        .errors
        .iter()
        .map(|found| found.message.as_str())
        .collect();
    assert_eq!(
        messages,
        vec![
            "Pipeline step 0 guardrail 'z' is not in the policy's guardrails.add list",
            "Pipeline step 0 guardrail 'z' not found in guardrail registry",
        ]
    );
}

#[test]
fn pipeline_valid_steps_and_absent_pipelines_report_nothing() {
    let all = policies(vec![
        ("with", pipeline_policy(&["a", "b"], &["b", "a"])),
        ("without", policy(None, &["a"], &[])),
    ]);
    assert!(
        offline(&["a", "b"])
            .validate_policies(&all, true)
            .errors
            .is_empty()
    );
}

#[test]
fn config_parse_validates_the_parsed_policies() {
    let raw = config(
        r#"{"base": {"guardrails": {"add": ["a"]}}, "child": {"inherit": "base", "guardrails": ["missing"]}}"#,
    );
    let response = offline(&["a"]).validate_policy_config(&raw, true);
    assert!(!response.valid);
    assert_eq!(
        response.errors,
        vec![expected(
            "child",
            PolicyValidationErrorType::InvalidGuardrail,
            "Guardrail 'missing' not found in guardrail registry",
            Some("guardrails.add"),
            Some("missing"),
        )]
    );
}

#[test]
fn config_parse_of_a_clean_config_is_valid() {
    let raw = config(
        r#"{"base": {"guardrails": {"add": ["a"], "remove": ["gone"]}}, "pipe": {"guardrails": {"add": ["a"]}, "pipeline": {"mode": "post_call", "steps": [{"guardrail": "a", "on_fail": "next"}]}}}"#,
    );
    let response = offline(&["a"]).validate_policy_config(&raw, false);
    assert!(response.valid);
    assert!(response.errors.is_empty());
    assert_eq!(response.warnings.len(), 1);
}

#[test]
fn config_parse_of_an_empty_config_is_valid() {
    let response = offline(&["a"]).validate_policy_config(&BTreeMap::new(), true);
    assert_eq!(
        response,
        PolicyValidationResponse {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new()
        }
    );
}

#[rstest]
#[case::unknown_key(r#"{"p": {"unknown": 1}}"#, "p", "unknown field `unknown`")]
#[case::wrong_type(r#"{"p": {"inherit": 5}}"#, "p", "invalid type")]
#[case::not_an_object(r#"{"p": "text"}"#, "p", "invalid type")]
#[case::null_policy(r#"{"p": null}"#, "p", "invalid type")]
#[case::bad_pipeline_mode(
    r#"{"p": {"pipeline": {"mode": "sideways", "steps": [{"guardrail": "a"}]}}}"#,
    "p",
    "Invalid mode 'sideways'"
)]
#[case::pipeline_without_steps(
    r#"{"p": {"pipeline": {"steps": []}}}"#,
    "p",
    "Invalid policy 'p': List should have at least 1 item after validation, not 0"
)]
fn parse_error_becomes_an_invalid_syntax_error(
    #[case] text: &str,
    #[case] policy_name: &str,
    #[case] reason: &str,
) {
    let response = offline(&["a"]).validate_policy_config(&config(text), true);
    assert!(!response.valid);
    assert!(response.warnings.is_empty());
    let [error] = response.errors.as_slice() else {
        panic!("expected one error, got {:?}", response.errors);
    };
    assert_eq!(error.policy_name, policy_name);
    assert_eq!(error.error_type, PolicyValidationErrorType::InvalidSyntax);
    assert!(
        error.message.starts_with("Failed to parse policy: "),
        "{}",
        error.message
    );
    assert!(error.message.contains(reason), "{}", error.message);
    assert_eq!(error.field, None);
    assert_eq!(error.value, None);
}

#[test]
fn parse_error_skips_validate_policies_and_reports_every_failure() {
    let raw = config(
        r#"{"a": {"guardrails": {"add": ["unregistered"]}, "inherit": "ghost"}, "b": {"nope": 1}, "c": "x"}"#,
    );
    let validator = offline(&["known"]);
    let response = validator.validate_policy_config(&raw, true);
    assert!(!response.valid);
    assert!(response.warnings.is_empty());
    let failed: Vec<&str> = response
        .errors
        .iter()
        .map(|found| found.policy_name.as_str())
        .collect();
    assert_eq!(failed, vec!["b", "c"]);
    assert_eq!(validator.guardrails.calls.load(Ordering::SeqCst), 0);
}
