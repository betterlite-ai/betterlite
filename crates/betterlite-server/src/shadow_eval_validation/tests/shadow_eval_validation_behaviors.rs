use std::error::Error as _;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rstest::rstest;

use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::router::JudgeRouter as _;
use crate::model_visibility::testing::{Row, Snapshot};
use crate::shadow_eval_validation::access::{AdminRole, require_admin};
use crate::shadow_eval_validation::error::AccessError;
use crate::shadow_eval_validation::request::{
    DEFAULT_JUDGE_MODEL, ShadowEvalDirection, parse_shadow_eval_request,
};
use crate::shadow_eval_validation::router_view::ShadowEvalRouter;
use crate::shadow_eval_validation::start::{TargetKey, TargetRecords};
use crate::team_access::error::TeamAccessError;

use super::support::{
    Credentials, JUDGE, JUDGE_MODEL, ROUTER, TIER, TIER_MODEL, auth, deployments, params, parse,
    records, router_params, validate, validate_with, validation_field,
};

#[test]
fn defaults_fill_every_optional_field() {
    let parsed = parse(
        r#"{"api_key_ids":["key-1"],"router_names":["shadow-router"],"shadow_percentage":50}"#,
    );
    assert_eq!(parsed.judge_model, DEFAULT_JUDGE_MODEL);
    assert_eq!(parsed.direction, ShadowEvalDirection::Forward);
    assert_eq!(parsed.duration_days, 7);
    assert_eq!(parsed.max_budget, rust_decimal::Decimal::new(100, 1));
    assert_eq!(parsed.baseline_model, None);
    assert_eq!(parsed.models.len(), 0);
}

#[test]
fn deduplicates_each_list_in_first_spelling_order() {
    let parsed = parse(
        r#"{"api_key_ids":["b","a","b"],"team_ids":["t","t"],"user_ids":["u","u"],"models":["m","m"],"router_names":["r2","r1","r2"],"shadow_percentage":50}"#,
    );
    assert_eq!(parsed.api_key_ids, ["b", "a"]);
    assert_eq!(parsed.team_ids, ["t"]);
    assert_eq!(parsed.user_ids, ["u"]);
    assert_eq!(parsed.models, ["m"]);
    assert_eq!(parsed.router_names, ["r2", "r1"]);
}

#[rstest]
#[case(33.333_333_f64, "33.33")]
#[case(0.1_f64, "0.10")]
#[case(100.0_f64, "100.00")]
fn rounds_the_percentage_to_two_decimals(#[case] given: f64, #[case] rounded: &str) {
    let body = format!(r#"{{"team_ids":["t"],"router_names":["r"],"shadow_percentage":{given}}}"#);
    let parsed = parse(&body);
    assert_eq!(format!("{:.2}", parsed.shadow_percentage), rounded);
}

#[test]
fn rejects_the_retired_max_turns_before_reading_fields() {
    let error =
        parse_shadow_eval_request(br#"{"max_turns":3,"shadow_percentage":0.01}"#).unwrap_err();
    assert_eq!(validation_field(&error), "max_turns");
}

#[rstest]
#[case(r#"{"team_ids":["t"],"router_name":"r","shadow_percentage":50}"#, vec!["r"])]
#[case(r#"{"team_ids":["t"],"router_names":["r2","r1","r2"],"shadow_percentage":50}"#, vec!["r2", "r1"])]
fn one_router_spelling_resolves_to_a_deduplicated_set(
    #[case] body: &str,
    #[case] routers: Vec<&str>,
) {
    let parsed = parse(body);
    let expected: Vec<String> = routers.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(parsed.router_names, expected);
}

#[rstest]
#[case(r#"{"team_ids":["t"],"router_name":"r","router_names":["r"],"shadow_percentage":50}"#)]
#[case(r#"{"team_ids":["t"],"shadow_percentage":50}"#)]
fn requires_exactly_one_router_spelling(#[case] body: &str) {
    let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
    assert_eq!(validation_field(&error), "router_names");
}

#[rstest]
#[case(
    r#"{"team_ids":["t"],"router_names":["r"],"direction":"reverse","shadow_percentage":50}"#,
    "baseline_model"
)]
#[case(
    r#"{"team_ids":["t"],"router_names":["r"],"baseline_model":"b","shadow_percentage":50}"#,
    "baseline_model"
)]
#[case(
    r#"{"team_ids":["t"],"router_names":["r"],"models":["m"],"direction":"reverse","baseline_model":"b","shadow_percentage":50}"#,
    "models"
)]
fn direction_decides_which_model_fields_apply(#[case] body: &str, #[case] field: &'static str) {
    let error = parse_shadow_eval_request(body.as_bytes()).unwrap_err();
    assert_eq!(validation_field(&error), field);
}

#[test]
fn reverse_jobs_evaluate_one_router() {
    let error = parse_shadow_eval_request(
        br#"{"team_ids":["t"],"router_names":["r1","r2"],"direction":"reverse","baseline_model":"b","shadow_percentage":50}"#,
    )
    .unwrap_err();
    assert_eq!(validation_field(&error), "router_names");
}

#[rstest]
#[case(None, false, false)]
#[case(Some("proxy_admin"), true, true)]
#[case(Some("proxy_admin_viewer"), false, true)]
#[case(Some("internal_user"), false, false)]
#[case(Some("customer"), false, false)]
#[case(Some("org_admin"), false, false)]
fn admin_roles_are_the_only_callers_that_pass(
    #[case] role: Option<&str>,
    #[case] writer: bool,
    #[case] viewer: bool,
) {
    let caller = auth(role);
    let writer_allowed = require_admin(&caller, "start", AdminRole::Writer).is_ok();
    let viewer_allowed = require_admin(&caller, "view", AdminRole::Viewer).is_ok();
    assert_eq!(writer_allowed, writer);
    assert_eq!(viewer_allowed, viewer);
}

#[test]
fn access_errors_carry_the_endpoint_status_codes_and_wording() {
    let caller = auth(Some("internal_user"));
    let writer = require_admin(&caller, "start a shadow eval", AdminRole::Writer).unwrap_err();
    let viewer = require_admin(&caller, "view shadow evals", AdminRole::Viewer).unwrap_err();
    assert_eq!(writer.status_code(), 403);
    assert_eq!(viewer.status_code(), 403);
    assert_eq!(
        writer.to_string(),
        "only a proxy admin can start a shadow eval"
    );
    assert_eq!(
        viewer.to_string(),
        "only proxy admin roles can view shadow evals"
    );
    let unreadable = AccessError::Caller(TeamAccessError::Caller(
        serde_json::from_str::<u8>("not a role").unwrap_err(),
    ));
    assert_eq!(unreadable.status_code(), 403);
    assert_eq!(
        unreadable.to_string(),
        "the caller's role could not be read"
    );
}

#[test]
fn start_validation_accepts_a_forward_job_on_a_configured_router() {
    let parsed = parse(
        r#"{"api_key_ids":["key-1"],"router_names":["shadow-router"],"judge_model":"judge-model","shadow_percentage":50}"#,
    );
    validate(&parsed, &deployments(), &records()).unwrap();
}

#[test]
fn start_validation_rejects_an_unconfigured_router_by_name() {
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["a-router","b-router"],"judge_model":"judge-model","shadow_percentage":50}"#,
    );
    let error = validate(&parsed, &deployments(), &records()).unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert_eq!(
        error.to_string(),
        "Not a configured auto-router: 'a-router', 'b-router'"
    );
}

#[test]
fn names_every_unknown_target_kind_in_one_message() {
    let parsed = parse(
        r#"{"api_key_ids":["zz-key"],"team_ids":["zz-team"],"user_ids":["zz-user"],"router_names":["shadow-router"],"judge_model":"judge-model","shadow_percentage":50}"#,
    );
    let error = validate(&parsed, &deployments(), &records()).unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert_eq!(
        error.to_string(),
        "api_key_ids not on this proxy: zz-key. team_ids not on this proxy: zz-team. user_ids not on this proxy: zz-user"
    );
}

#[test]
fn rejects_a_judge_that_another_deployment_also_serves() {
    let mut rows = deployments();
    rows.push(("judge-alias", params(TIER_MODEL)));
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"judge_model":"judge-alias","shadow_percentage":50}"#,
    );
    let error = validate(&parsed, &rows, &records()).unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert!(
        error
            .to_string()
            .contains("judge_model 'judge-alias' is also an arm this job would judge")
    );
}

#[test]
fn rejects_an_auto_router_as_a_plain_model() {
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"judge_model":"shadow-router","shadow_percentage":50}"#,
    );
    let error = validate(&parsed, &deployments(), &records()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "judge_model 'shadow-router' is an auto-router; it must be a plain model"
    );
}

#[test]
fn an_alias_does_not_configure_its_target_as_a_router() {
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["smart"],"judge_model":"judge-model","shadow_percentage":50}"#,
    );
    let aliases = JsonObject::from([("smart".to_owned(), Json::Text(ROUTER.to_owned()))]);
    let error = validate_with(
        &parsed,
        &deployments(),
        aliases,
        &records(),
        &Credentials::None,
    )
    .unwrap_err();
    assert_eq!(error.status_code(), 400);
    assert_eq!(error.to_string(), "Not a configured auto-router: 'smart'");
}

#[test]
fn an_alias_naming_a_router_is_still_a_plain_model() {
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"judge_model":"smart","shadow_percentage":50}"#,
    );
    let aliases = JsonObject::from([("smart".to_owned(), Json::Text(ROUTER.to_owned()))]);
    validate_with(
        &parsed,
        &deployments(),
        aliases,
        &records(),
        &Credentials::None,
    )
    .unwrap();
}

#[test]
fn a_malformed_alias_fails_the_start_instead_of_reading_no_arms() {
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"judge_model":"judge-model","shadow_percentage":50}"#,
    );
    let aliases = JsonObject::from([(ROUTER.to_owned(), Json::Object(JsonObject::new()))]);
    let error = validate_with(
        &parsed,
        &deployments(),
        aliases,
        &records(),
        &Credentials::None,
    )
    .unwrap_err();
    assert_eq!(error.status_code(), 500);
    assert!(
        error
            .to_string()
            .starts_with("the router's arm models could not be read")
    );
    assert!(error.source().is_some());
}

#[test]
fn rejects_a_scope_no_request_could_name() {
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"models":["not-a-model"],"judge_model":"judge-model","shadow_percentage":50}"#,
    );
    let error = validate(&parsed, &deployments(), &records()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "models not served by this proxy: 'not-a-model'"
    );
}

#[test]
fn accepts_a_provider_qualified_public_model_in_the_scope() {
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"models":["anthropic/claude-sonnet-5"],"judge_model":"judge-model","shadow_percentage":50}"#,
    );
    validate(&parsed, &deployments(), &records()).unwrap();
}

#[test]
fn rejects_an_sdk_judge_without_anthropic_credentials() {
    let parsed = parse(
        r#"{"user_ids":["user-1"],"router_names":["shadow-router"],"judge_model":"anthropic/claude-sonnet-5","shadow_percentage":50}"#,
    );
    let error = validate(&parsed, &deployments(), &records()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "judge_model 'anthropic/claude-sonnet-5' needs ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN"
    );
}

#[test]
fn names_the_teams_a_fault_applies_to_in_sorted_order() {
    let keys = [TargetKey {
        token: "key-1",
        team_id: Some("team-c"),
    }];
    let records = TargetRecords {
        keys: &keys,
        teams: &["team-a", "team-b"],
        users: &["user-1"],
    };
    let parsed = parse(
        r#"{"api_key_ids":["key-1"],"team_ids":["team-a","team-b"],"user_ids":["user-1"],"router_names":["shadow-router"],"judge_model":"anthropic/claude-sonnet-5","shadow_percentage":50}"#,
    );
    let error = validate(&parsed, &deployments(), &records).unwrap_err();
    assert_eq!(
        error.to_string(),
        "judge_model 'anthropic/claude-sonnet-5' needs ANTHROPIC_API_KEY or ANTHROPIC_AUTH_TOKEN for team team-a, team-b, team-c"
    );
}

#[test]
fn a_router_served_judge_needs_no_anthropic_credentials() {
    let rows = vec![
        (ROUTER, router_params("other-model")),
        ("other-model", params("openai/gpt-4o")),
        ("anthropic/claude-sonnet-5", params(JUDGE_MODEL)),
    ];
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"judge_model":"anthropic/claude-sonnet-5","shadow_percentage":50}"#,
    );
    validate(&parsed, &rows, &records()).unwrap();
}

#[test]
fn accepts_an_sdk_judge_once_a_key_is_configured() {
    let rows = vec![
        (ROUTER, router_params("other-model")),
        ("other-model", params("openai/gpt-4o")),
    ];
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"judge_model":"anthropic/claude-sonnet-5","shadow_percentage":50}"#,
    );
    validate_with(
        &parsed,
        &rows,
        JsonObject::new(),
        &records(),
        &Credentials::ApiKey("sk-ant-test".to_owned()),
    )
    .unwrap();
}

fn tiered_deployments() -> Vec<(&'static str, JsonObject)> {
    let tiers = JsonObject::from([("SIMPLE".to_owned(), Json::Text(TIER.to_owned()))]);
    let compaction = JsonObject::from([("model".to_owned(), Json::Text(JUDGE.to_owned()))]);
    let config = JsonObject::from([
        ("tiers".to_owned(), Json::Object(tiers)),
        ("context_compaction".to_owned(), Json::Object(compaction)),
    ]);
    let mut router = router_params("other-model");
    router.insert("complexity_router_config".to_owned(), Json::Object(config));
    vec![
        (JUDGE, params(JUDGE_MODEL)),
        (TIER, params(TIER_MODEL)),
        (ROUTER, router),
        ("other-model", params("openai/gpt-4o")),
    ]
}

#[rstest]
#[case(TIER, Some("tier model 'tier-model'"))]
#[case("other-model", Some("default model 'other-model'"))]
#[case(JUDGE, None)]
fn only_tier_and_default_arms_collide_with_the_judge(
    #[case] judge: &str,
    #[case] collision: Option<&str>,
) {
    let body = format!(
        r#"{{"team_ids":["team-1"],"router_names":["shadow-router"],"judge_model":"{judge}","shadow_percentage":50}}"#
    );
    let result = validate(&parse(&body), &tiered_deployments(), &records());
    match collision {
        Some(arm) => assert!(result.unwrap_err().to_string().contains(arm)),
        None => result.unwrap(),
    }
}

#[tokio::test]
async fn the_validation_router_refuses_to_dispatch_a_judge_call() {
    let rows: Vec<Row<'_>> = Vec::new();
    let fixture = Snapshot::build(&rows, &JsonObject::new()).unwrap();
    let provider = |_model: &str| -> Option<String> { None };
    let router = ShadowEvalRouter {
        catalog: &fixture.catalog,
        model_group_alias: &fixture.aliases,
        routing_groups: &fixture.groups,
        inferred_provider: &provider,
    };
    let call = JudgeCall {
        model: "judge-model".to_owned(),
        messages: Vec::new(),
        num_retries: 0,
        fallbacks: None,
        drop_params: false,
        params: IndexMap::new(),
    };
    let error = router.acompletion(call).await.unwrap_err();
    assert_eq!(
        error.message,
        "start-time validation resolves models without dispatching a judge call"
    );
}

#[rstest]
#[case("ANTHROPIC_API_KEY", true)]
#[case("os.environ/ANTHROPIC_AUTH_TOKEN", true)]
#[case("OPENAI_API_KEY", false)]
fn a_hosted_manager_holding_either_anthropic_secret_is_consulted(
    #[case] hosted: &str,
    #[case] accepted: bool,
) {
    let rows = vec![
        (ROUTER, router_params("other-model")),
        ("other-model", params("openai/gpt-4o")),
    ];
    let parsed = parse(
        r#"{"team_ids":["team-1"],"router_names":["shadow-router"],"judge_model":"anthropic/claude-sonnet-5","shadow_percentage":50}"#,
    );
    let keys = Credentials::Hosted(vec![hosted.to_owned()]);
    let result = validate_with(&parsed, &rows, JsonObject::new(), &records(), &keys);
    assert_eq!(result.is_ok(), accepted);
}
