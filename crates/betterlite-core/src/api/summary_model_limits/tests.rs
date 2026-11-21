#![cfg(test)]

mod real_limiter;
mod support;

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;
use serde_json::json;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::registry;

use super::budget::summary_model_within_budget;
use super::rate::{
    LimiterCapability, PARALLEL_REQUEST_LIMITER, SummaryDescriptor, summary_model_within_rate_limit,
};
use support::{
    BudgetCall, FakeBudgets, FakeHooks, FakeLimiter, LimiterCall, RateFailure, Stage, Verdict,
    Warnings, budget, descriptor, principal,
};

const MODEL: &str = "sum";

fn every_scope() -> serde_json::Value {
    json!({
        "token": "key-1",
        "key_alias": "alias-1",
        "model_max_budget": {"sum": {"budget_limit": 1.0, "time_period": "1d"}},
        "user_id": "user-1",
        "user_model_max_budget": {"sum": {"budget_limit": 1.0, "time_period": "1d"}},
        "team_id": "team-1",
        "team_model_max_budget": {"sum": {"budget_limit": 1.0, "time_period": "1d"}},
        "end_user_id": "end-1",
        "end_user_model_max_budget": {"sum": {"budget_limit": 1.0, "time_period": "1d"}},
    })
}

fn call(scope: &'static str, id: &str) -> BudgetCall {
    BudgetCall {
        scope,
        id: id.to_owned(),
        alias: (scope == "key").then(|| "alias-1".to_owned()),
        key_budget: (scope == "team").then(|| budget(MODEL)),
        budget: budget(MODEL),
        model: MODEL.to_owned(),
    }
}

fn proxy_with(limiter: Option<FakeLimiter>) -> FakeHooks {
    FakeHooks {
        limiter,
        raises: false,
        requested: Mutex::new(Vec::new()),
    }
}

#[tokio::test]
async fn budget_scope_arguments_check_key_user_team_then_end_user() {
    let budgets = FakeBudgets::default();
    let auth = principal(every_scope());
    let allowed = summary_model_within_budget(Some(&auth), Some(&budgets), MODEL).await;
    assert!(allowed.unwrap());
    let expected = vec![
        call("key", "key-1"),
        call("user", "user-1"),
        call("team", "team-1"),
        call("end-user", "end-1"),
    ];
    assert_eq!(budgets.calls(), expected);
}

#[tokio::test]
async fn budget_scope_arguments_skip_empty_budgets_and_null_ids() {
    let budgets = FakeBudgets::default();
    let auth = principal(json!({
        "token": null,
        "model_max_budget": {"sum": {"budget_limit": 1.0}},
        "user_id": "user-1",
        "user_model_max_budget": {},
        "team_id": null,
        "team_model_max_budget": {"sum": {"budget_limit": 1.0}},
        "end_user_id": "end-1",
        "end_user_model_max_budget": {},
    }));
    let allowed = summary_model_within_budget(Some(&auth), Some(&budgets), MODEL).await;
    assert!(allowed.unwrap());
    assert_eq!(budgets.calls(), Vec::new());
}

#[tokio::test]
async fn budget_scope_arguments_check_empty_string_ids() {
    let budgets = FakeBudgets::default();
    let mut fields = every_scope();
    fields["token"] = json!("");
    fields["user_id"] = json!("");
    fields["team_id"] = json!("");
    fields["end_user_id"] = json!("");
    let auth = principal(fields);
    let allowed = summary_model_within_budget(Some(&auth), Some(&budgets), MODEL).await;
    assert!(allowed.unwrap());
    let ids: Vec<String> = budgets.calls().into_iter().map(|call| call.id).collect();
    assert_eq!(ids, vec![String::new(); 4]);
}

#[tokio::test]
async fn budget_scope_arguments_pass_an_empty_key_budget_to_the_team() {
    let budgets = FakeBudgets::default();
    let mut fields = every_scope();
    fields["model_max_budget"] = json!({});
    let auth = principal(fields);
    let allowed = summary_model_within_budget(Some(&auth), Some(&budgets), MODEL).await;
    assert!(allowed.unwrap());
    let team = budgets
        .calls()
        .into_iter()
        .find(|call| call.scope == "team");
    assert_eq!(team.unwrap().key_budget, Some(BTreeMap::new()));
}

#[tokio::test]
async fn budget_scope_arguments_allow_without_a_principal_or_a_limiter() {
    let budgets = FakeBudgets::default();
    let without_principal = summary_model_within_budget(None, Some(&budgets), MODEL).await;
    assert!(without_principal.unwrap());
    let auth = principal(every_scope());
    let without_limiter =
        summary_model_within_budget::<FakeBudgets>(Some(&auth), None, MODEL).await;
    assert!(without_limiter.unwrap());
    assert_eq!(budgets.calls(), Vec::new());
}

#[rstest]
#[case::key("key", 1)]
#[case::user("user", 2)]
#[case::team("team", 3)]
#[case::end_user("end-user", 4)]
#[tokio::test]
async fn budget_errors_stop_later_scopes(
    #[case] scope: &'static str,
    #[case] reached: usize,
    #[values(Verdict::Exceeded, Verdict::Offline)] verdict: Verdict,
) {
    let warnings = Warnings::default();
    let _guard = set_default(registry().with(warnings.clone()));
    let budgets = FakeBudgets {
        failing: Some((scope, verdict)),
        calls: Mutex::new(Vec::new()),
    };
    let auth = principal(every_scope());
    let allowed = summary_model_within_budget(Some(&auth), Some(&budgets), MODEL).await;
    assert!(!allowed.unwrap());
    assert_eq!(budgets.calls().len(), reached);
    let expected = match verdict {
        Verdict::Exceeded => Vec::new(),
        Verdict::Offline => vec![format!(
            "compact_20260112: unexpected error during {scope} model-budget check for summary_model=sum; denying: offline"
        )],
    };
    assert_eq!(warnings.lines(), expected);
}

fn rate_principal() -> serde_json::Value {
    json!({
        "token": "key-1",
        "metadata": {"rpm_limit_type": "dynamic", "tpm_limit_type": "guaranteed_throughput"},
        "parent_otel_span": {"span": "parent"},
    })
}

fn gauged(key: &str) -> SummaryDescriptor {
    descriptor(
        key,
        Some(
            json!({"requests_per_unit": 10, "tokens_per_unit": 20, "max_parallel_requests": 1, "window_size": 60}),
        ),
    )
}

fn ungauged(key: &str) -> SummaryDescriptor {
    descriptor(
        key,
        Some(json!({"requests_per_unit": 10, "tokens_per_unit": 20, "window_size": 60})),
    )
}

fn limiter_with(base: Vec<SummaryDescriptor>, overall_code: &str) -> FakeLimiter {
    FakeLimiter {
        missing: Vec::new(),
        base,
        team: vec![gauged("model_per_team")],
        project: vec![gauged("model_per_project")],
        organization: vec![gauged("organization")],
        overall_code: Some(overall_code.to_owned()),
        failure: None,
        calls: Mutex::new(Vec::new()),
    }
}

async fn rate_check(hooks: &FakeHooks, fields: serde_json::Value) -> bool {
    let auth = principal(fields);
    summary_model_within_rate_limit(Some(&auth), Some(hooks), MODEL)
        .await
        .unwrap()
}

#[tokio::test]
async fn rate_descriptor_order_builds_then_appends_team_project_and_organization() {
    let hooks = proxy_with(Some(limiter_with(vec![gauged("api_key")], "OK")));
    assert!(rate_check(&hooks, rate_principal()).await);
    assert_eq!(
        *hooks.requested.lock().unwrap(),
        vec![PARALLEL_REQUEST_LIMITER.to_owned()]
    );
    let token = Some("key-1".to_owned());
    let expected = vec![
        LimiterCall::Create {
            token: token.clone(),
            model: MODEL.to_owned(),
            rpm_limit_type: Some(JsonData::String("dynamic".to_owned())),
            tpm_limit_type: Some(JsonData::String("guaranteed_throughput".to_owned())),
            model_has_failures: false,
        },
        LimiterCall::Team {
            token: token.clone(),
            model: MODEL.to_owned(),
            seen: vec![gauged("api_key")],
        },
        LimiterCall::Project {
            token: token.clone(),
            model: MODEL.to_owned(),
            seen: vec![gauged("api_key"), gauged("model_per_team")],
        },
        LimiterCall::Organization {
            token,
            model: MODEL.to_owned(),
        },
        LimiterCall::Check {
            descriptors: vec![
                ungauged("api_key"),
                ungauged("model_per_team"),
                ungauged("model_per_project"),
                ungauged("organization"),
            ],
            parent_otel_span: Some(JsonData::Object(BTreeMap::from([(
                "span".to_owned(),
                JsonData::String("parent".to_owned()),
            )]))),
            read_only: true,
        },
    ];
    assert_eq!(hooks.limiter.unwrap().calls(), expected);
}

#[tokio::test]
async fn rate_descriptor_order_allows_empty_descriptors_without_a_check() {
    let mut limiter = limiter_with(Vec::new(), "OVER_LIMIT");
    limiter.team.clear();
    limiter.project.clear();
    limiter.organization.clear();
    let hooks = proxy_with(Some(limiter));
    assert!(rate_check(&hooks, rate_principal()).await);
    let calls = hooks.limiter.unwrap().calls();
    assert_eq!(calls.len(), 4);
    assert!(
        !calls
            .iter()
            .any(|call| matches!(call, LimiterCall::Check { .. }))
    );
}

#[rstest]
#[case::missing_rate_limit(descriptor("plain", None), descriptor("plain", None))]
#[case::null_gauge(ungauged("api_key"), ungauged("api_key"))]
#[case::zero_gauge(
    descriptor("api_key", Some(json!({"requests_per_unit": 5, "max_parallel_requests": 0}))),
    descriptor("api_key", Some(json!({"requests_per_unit": 5})))
)]
#[case::one_gauge(gauged("api_key"), ungauged("api_key"))]
#[case::only_a_gauge(
    descriptor("api_key", Some(json!({"max_parallel_requests": 3}))),
    descriptor("api_key", Some(json!({})))
)]
#[tokio::test]
async fn parallel_gauge_filter(
    #[case] built: SummaryDescriptor,
    #[case] checked: SummaryDescriptor,
) {
    let limiter = limiter_with(vec![built.clone(), descriptor("second", None)], "OK");
    let hooks = proxy_with(Some(limiter));
    assert!(rate_check(&hooks, rate_principal()).await);
    let calls = hooks.limiter.unwrap().calls();
    let Some(LimiterCall::Project { seen, .. }) = calls.get(2) else {
        panic!("the project descriptor was not added: {calls:?}");
    };
    let unchanged = vec![built, descriptor("second", None), gauged("model_per_team")];
    assert_eq!(*seen, unchanged);
    let Some(LimiterCall::Check { descriptors, .. }) = calls.last() else {
        panic!("the read-only check did not run: {calls:?}");
    };
    let mut expected = vec![checked, descriptor("second", None)];
    expected.extend([
        ungauged("model_per_team"),
        ungauged("model_per_project"),
        ungauged("organization"),
    ]);
    assert_eq!(*descriptors, expected);
}

#[rstest]
#[case::should_rate_limit(LimiterCapability::ShouldRateLimit)]
#[case::create_descriptors(LimiterCapability::CreateDescriptors)]
#[case::team_descriptor(LimiterCapability::AddTeamModelDescriptor)]
#[case::project_descriptor(LimiterCapability::AddProjectModelDescriptor)]
#[case::organization_descriptors(LimiterCapability::CreateOrganizationDescriptors)]
#[tokio::test]
async fn unavailable_rate_capabilities_allow_without_calls(#[case] missing: LimiterCapability) {
    let mut limiter = limiter_with(vec![gauged("api_key")], "OVER_LIMIT");
    limiter.missing = vec![missing];
    let hooks = proxy_with(Some(limiter));
    assert!(rate_check(&hooks, rate_principal()).await);
    assert_eq!(hooks.limiter.unwrap().calls(), Vec::new());
}

#[tokio::test]
async fn unavailable_rate_capabilities_allow_without_principal_proxy_or_hook() {
    let auth = principal(rate_principal());
    let registered = proxy_with(Some(limiter_with(vec![gauged("api_key")], "OVER_LIMIT")));
    let without_principal = summary_model_within_rate_limit(None, Some(&registered), MODEL).await;
    assert!(without_principal.unwrap());
    assert_eq!(registered.requested.lock().unwrap().len(), 0);
    let without_proxy =
        summary_model_within_rate_limit::<FakeHooks>(Some(&auth), None, MODEL).await;
    assert!(without_proxy.unwrap());
    let unregistered = proxy_with(None);
    assert!(rate_check(&unregistered, rate_principal()).await);
    assert_eq!(unregistered.requested.lock().unwrap().len(), 1);
    assert_eq!(registered.limiter.unwrap().calls(), Vec::new());
}

#[tokio::test]
async fn unavailable_rate_capabilities_read_a_missing_metadata_map_as_empty() {
    let hooks = proxy_with(Some(limiter_with(vec![gauged("api_key")], "OK")));
    assert!(rate_check(&hooks, json!({"token": "key-1"})).await);
    let calls = hooks.limiter.unwrap().calls();
    let Some(LimiterCall::Create {
        rpm_limit_type,
        tpm_limit_type,
        ..
    }) = calls.first()
    else {
        panic!("the base descriptors were not built: {calls:?}");
    };
    assert_eq!((rpm_limit_type, tpm_limit_type), (&None, &None));
}

#[tokio::test]
async fn unavailable_rate_capabilities_propagate_a_hook_lookup_failure() {
    let mut registered = proxy_with(Some(limiter_with(vec![gauged("api_key")], "OK")));
    registered.raises = true;
    let auth = principal(rate_principal());
    let failed = summary_model_within_rate_limit(Some(&auth), Some(&registered), MODEL).await;
    assert_eq!(
        failed.unwrap_err().to_string(),
        "the parallel request limiter hook lookup failed"
    );
    assert_eq!(registered.limiter.unwrap().calls(), Vec::new());
}

#[rstest]
#[case::over_limit(Some("OVER_LIMIT"), false)]
#[case::ok(Some("OK"), true)]
#[case::other_code(Some("over_limit"), true)]
#[case::missing_code(None, true)]
#[tokio::test]
async fn rate_denial_and_failures_deny_only_over_limit(
    #[case] overall_code: Option<&str>,
    #[case] allowed: bool,
) {
    let mut limiter = limiter_with(vec![gauged("api_key")], "OK");
    limiter.overall_code = overall_code.map(str::to_owned);
    let hooks = proxy_with(Some(limiter));
    assert_eq!(rate_check(&hooks, rate_principal()).await, allowed);
}

#[rstest]
#[case::unverified_build(Stage::Create, RateFailure::Unverifiable, false, 1)]
#[case::unverified_check(Stage::Check, RateFailure::Unverifiable, false, 5)]
#[case::offline_build(Stage::Create, RateFailure::Offline, true, 1)]
#[case::offline_team(Stage::Team, RateFailure::Offline, true, 2)]
#[case::offline_project(Stage::Project, RateFailure::Offline, true, 3)]
#[case::offline_organization(Stage::Organization, RateFailure::Offline, true, 4)]
#[case::offline_check(Stage::Check, RateFailure::Offline, true, 5)]
#[tokio::test]
async fn rate_denial_and_failures(
    #[case] stage: Stage,
    #[case] failure: RateFailure,
    #[case] allowed: bool,
    #[case] reached: usize,
) {
    let warnings = Warnings::default();
    let _guard = set_default(registry().with(warnings.clone()));
    let mut limiter = limiter_with(vec![gauged("api_key")], "OVER_LIMIT");
    limiter.failure = Some((stage, failure));
    let hooks = proxy_with(Some(limiter));
    assert_eq!(rate_check(&hooks, rate_principal()).await, allowed);
    assert_eq!(hooks.limiter.unwrap().calls().len(), reached);
    let expected = match failure {
        RateFailure::Unverifiable => {
            "compact_20260112: rate-limit counters for summary_model=sum could not be verified; denying: unverified"
        }
        RateFailure::Offline => {
            "compact_20260112: unexpected error during rate-limit check for summary_model=sum; allowing: offline"
        }
    };
    assert_eq!(warnings.lines(), vec![expected.to_owned()]);
}
