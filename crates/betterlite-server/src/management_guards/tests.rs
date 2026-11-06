#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::{
    CredentialRef, NetworkContext, Principal, SourceKey, UserIdentity,
};
use betterlite_auth::auth_identity::roles::{AuthMethod, PrincipalType, Role};
use betterlite_config::sections::budget_reset::BudgetResetSettings;
use betterlite_core::budget_reset::ResetClocks;
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use rust_decimal::Decimal;

use super::amounts::{Amount, validate_finite_spend};
use super::caller::{
    caller_user_id_for_non_admin, check_guardrail_opt_out, check_passthrough_routes,
};
use super::duration::validate_budget_duration;
use super::error::ManagementGuardError;
use super::metadata::{set_object_metadata_field, update_metadata_field, update_metadata_fields};
use super::team_budget::{ModelBudgetConfig, validate_team_model_max_budget};

type NamedConfig = (&'static str, ModelBudgetConfig<'static>);

const DATABASE: fn() -> TimeZoneDatabase = TimeZoneDatabase::from_env;
const INVALID_DURATION: &str =
    "invalid budget_duration: use a format like '1h', '24h', '7d', or '30d'";

fn settings() -> BudgetResetSettings {
    BudgetResetSettings::default()
}

fn clocks() -> ResetClocks<'static> {
    let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    ResetClocks {
        reset_now: now,
        calendar_now: now,
        calendar_timezone: None,
    }
}

fn principal(user_id: Option<&str>, roles: &[Role]) -> Principal {
    Principal {
        principal_type: PrincipalType::Human,
        subject: user_id.unwrap_or("service-account").to_owned(),
        issuer: None,
        audience: Vec::new(),
        user: user_id.map(|id| UserIdentity {
            id: id.to_owned(),
            external_id: None,
            user_name: None,
            email: None,
            display_name: None,
        }),
        organization: None,
        teams: Vec::new(),
        project: None,
        end_user: None,
        roles: roles.to_vec(),
        scopes: Vec::new(),
        auth_method: AuthMethod::ApiKey,
        credential_ref: CredentialRef::default(),
        network: NetworkContext::default(),
        source_key: SourceKey::default(),
    }
}

fn value(text: &str) -> JsonData {
    JsonData::String(text.to_owned())
}

fn number(value: i64) -> JsonData {
    JsonData::Number(serde_json::Number::from(value))
}

fn object(entries: &[(&str, JsonData)]) -> JsonData {
    JsonData::Object(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn config(
    max_budget: Option<Amount>,
    duration: Option<&'static str>,
) -> ModelBudgetConfig<'static> {
    ModelBudgetConfig {
        max_budget,
        budget_duration: duration,
        tpm_limit: None,
        rpm_limit: None,
    }
}

fn budget_map(entries: &[NamedConfig]) -> BTreeMap<String, ModelBudgetConfig<'static>> {
    entries
        .iter()
        .map(|(name, config)| ((*name).to_owned(), *config))
        .collect()
}

#[rstest]
#[case::absent(None)]
#[case::zero(Some(Amount::Finite(Decimal::ZERO)))]
#[case::negative(Some(Amount::Finite(Decimal::new(-15, 1))))]
#[case::positive(Some(Amount::Finite(Decimal::from(10_i32))))]
fn finite_spend_accepted(#[case] spend: Option<Amount>) {
    assert_eq!(validate_finite_spend(spend), Ok(()));
}

#[rstest]
#[case::nan(Amount::Nan, "spend must be a finite number. Received: nan")]
#[case::positive_infinity(
    Amount::PositiveInfinity,
    "spend must be a finite number. Received: inf"
)]
#[case::negative_infinity(
    Amount::NegativeInfinity,
    "spend must be a finite number. Received: -inf"
)]
fn nonfinite_spend(#[case] kind: Amount, #[case] message: &str) {
    let error = validate_finite_spend(Some(kind)).unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::non_positive("0d")]
#[case::unparsable("nonsense")]
fn invalid_duration(#[case] duration: &str) {
    let defaults = settings();
    let error = validate_budget_duration(
        Some(duration),
        StatusCode::BAD_REQUEST,
        &defaults,
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(error.to_string(), INVALID_DURATION);
    assert_eq!(
        validate_budget_duration(
            None,
            StatusCode::BAD_REQUEST,
            &defaults,
            clocks(),
            &DATABASE()
        ),
        Ok(())
    );
    assert_eq!(
        validate_budget_duration(
            Some("30d"),
            StatusCode::BAD_REQUEST,
            &defaults,
            clocks(),
            &DATABASE()
        ),
        Ok(())
    );
}

#[test]
fn duration_status_override() {
    let defaults = settings();
    let error = validate_budget_duration(
        Some("nonsense"),
        StatusCode::UNPROCESSABLE_ENTITY,
        &defaults,
        clocks(),
        &DATABASE(),
    )
    .unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::UNPROCESSABLE_ENTITY));
    assert_eq!(error.to_string(), INVALID_DURATION);
}

#[test]
fn anonymous_analytics() {
    let error = caller_user_id_for_non_admin(&principal(None, &[])).unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::FORBIDDEN));
    assert_eq!(
        error.to_string(),
        "Service-account keys cannot query user analytics. Use a user-bound key, or call as a proxy admin."
    );
    let admin = principal(None, &[Role::PlatformAdmin]);
    assert_eq!(
        caller_user_id_for_non_admin(&admin).unwrap_err(),
        ManagementGuardError::ServiceAccountAnalytics
    );
    assert_eq!(
        caller_user_id_for_non_admin(&principal(Some(""), &[])),
        Ok("")
    );
    assert_eq!(
        caller_user_id_for_non_admin(&principal(Some("u1"), &[])),
        Ok("u1")
    );
}

#[test]
fn passthrough_metadata_denied() {
    let caller = principal(Some("u1"), &[]);
    let metadata = object(&[(
        "allowed_passthrough_routes",
        JsonData::Array(vec![value("/v1/files")]),
    )]);
    let error = check_passthrough_routes(&caller, None, Some(&metadata), "key").unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::FORBIDDEN));
    assert_eq!(
        error.to_string(),
        "Only proxy admins can set `metadata.allowed_passthrough_routes` on a key."
    );
    let cleared = object(&[("allowed_passthrough_routes", JsonData::Array(Vec::new()))]);
    assert_eq!(
        check_passthrough_routes(&caller, None, Some(&cleared), "key"),
        Ok(())
    );
    assert_eq!(
        check_passthrough_routes(&caller, None, Some(&value("not-a-dict")), "key"),
        Ok(())
    );
}

#[test]
fn passthrough_top_level_before_metadata() {
    let caller = principal(Some("u1"), &[]);
    let metadata = object(&[(
        "allowed_passthrough_routes",
        JsonData::Array(vec![value("/y")]),
    )]);
    let routes = [value("/x")];
    let error =
        check_passthrough_routes(&caller, Some(&routes), Some(&metadata), "team").unwrap_err();
    assert_eq!(
        error.to_string(),
        "Only proxy admins can set `allowed_passthrough_routes` on a team."
    );
    let admin = principal(Some("u1"), &[Role::PlatformAdmin]);
    assert_eq!(
        check_passthrough_routes(&admin, Some(&routes), Some(&metadata), "team"),
        Ok(())
    );
    let viewer = principal(Some("u1"), &[Role::PlatformViewer]);
    assert_eq!(
        check_passthrough_routes(&viewer, Some(&routes), None, "team"),
        Err(ManagementGuardError::PassthroughRoutes {
            entity: "team".to_owned()
        })
    );
}

#[test]
fn guardrail_opt_out_denied() {
    let caller = principal(Some("u1"), &[]);
    let error = check_guardrail_opt_out(&caller, Some(true), None, None, "team").unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::FORBIDDEN));
    assert_eq!(
        error.to_string(),
        "Only proxy admins can set `disable_global_guardrails` on a team."
    );
    assert_eq!(
        check_guardrail_opt_out(&caller, Some(false), None, None, "team"),
        Ok(())
    );
    assert_eq!(
        check_guardrail_opt_out(&caller, None, None, None, "team"),
        Ok(())
    );
    let flag = object(&[("disable_global_guardrails", JsonData::Bool(true))]);
    assert_eq!(
        check_guardrail_opt_out(&caller, None, Some(&flag), None, "team"),
        Err(ManagementGuardError::GuardrailsDisabled {
            entity: "team".to_owned()
        })
    );
    let viewer = principal(Some("u1"), &[Role::PlatformViewer]);
    assert_eq!(
        check_guardrail_opt_out(&viewer, Some(true), None, None, "team"),
        Err(ManagementGuardError::GuardrailsDisabled {
            entity: "team".to_owned()
        })
    );
    let admin = principal(Some("u1"), &[Role::PlatformAdmin]);
    assert_eq!(
        check_guardrail_opt_out(&admin, Some(true), None, None, "team"),
        Ok(())
    );
}

#[test]
fn guardrail_existing_exact_true() {
    let caller = principal(Some("u1"), &[]);
    let stored_true = object(&[("disable_global_guardrails", JsonData::Bool(true))]);
    assert_eq!(
        check_guardrail_opt_out(&caller, Some(true), None, Some(&stored_true), "key"),
        Ok(())
    );
    let stored_one = object(&[("disable_global_guardrails", number(1))]);
    assert_eq!(
        check_guardrail_opt_out(&caller, Some(true), None, Some(&stored_one), "key"),
        Err(ManagementGuardError::GuardrailsDisabled {
            entity: "key".to_owned()
        })
    );
    let stored_text = object(&[("disable_global_guardrails", value("true"))]);
    assert_eq!(
        check_guardrail_opt_out(&caller, Some(true), None, Some(&stored_text), "key"),
        Err(ManagementGuardError::GuardrailsDisabled {
            entity: "key".to_owned()
        })
    );
}

#[test]
fn empty_metadata_clear() {
    let mut updated_kv = BTreeMap::from([
        ("tags".to_owned(), JsonData::Array(Vec::new())),
        (
            "metadata".to_owned(),
            object(&[("tags", JsonData::Array(vec![value("old")]))]),
        ),
    ]);
    assert_eq!(update_metadata_field(&mut updated_kv, "tags"), Ok(()));
    assert_eq!(
        updated_kv.get("metadata"),
        Some(&object(&[("tags", JsonData::Array(Vec::new()))]))
    );
    assert!(!updated_kv.contains_key("tags"));
}

#[test]
fn metadata_null_stays_top_level() {
    let mut updated_kv = BTreeMap::from([
        ("tags".to_owned(), JsonData::Null),
        (
            "metadata".to_owned(),
            object(&[("tags", JsonData::Array(vec![value("old")]))]),
        ),
    ]);
    assert_eq!(update_metadata_field(&mut updated_kv, "tags"), Ok(()));
    assert_eq!(updated_kv.get("tags"), Some(&JsonData::Null));
    assert_eq!(
        updated_kv.get("metadata"),
        Some(&object(&[("tags", JsonData::Array(vec![value("old")]))]))
    );
}

#[rstest]
#[case::false_flag(JsonData::Bool(false))]
#[case::empty_array(JsonData::Array(Vec::new()))]
#[case::empty_object(JsonData::Object(BTreeMap::new()))]
#[case::zero(number(0))]
#[case::empty_text(JsonData::String(String::new()))]
fn metadata_falsy_values_move(#[case] falsy: JsonData) {
    let mut updated_kv = BTreeMap::from([("enforced_params".to_owned(), falsy.clone())]);
    assert_eq!(
        update_metadata_field(&mut updated_kv, "enforced_params"),
        Ok(())
    );
    assert_eq!(updated_kv.get("enforced_params"), None);
    assert_eq!(
        updated_kv.get("metadata"),
        Some(&object(&[("enforced_params", falsy)]))
    );
}

#[test]
fn metadata_overwrite() {
    let mut updated_kv = BTreeMap::from([
        ("tags".to_owned(), JsonData::Array(vec![value("new")])),
        (
            "metadata".to_owned(),
            object(&[("tags", JsonData::Array(vec![value("old")]))]),
        ),
    ]);
    assert_eq!(update_metadata_field(&mut updated_kv, "tags"), Ok(()));
    assert_eq!(
        updated_kv.get("metadata"),
        Some(&object(&[("tags", JsonData::Array(vec![value("new")]))]))
    );
    let mut without_metadata =
        BTreeMap::from([("tags".to_owned(), JsonData::Array(vec![value("new")]))]);
    assert_eq!(update_metadata_field(&mut without_metadata, "tags"), Ok(()));
    assert_eq!(
        without_metadata.get("metadata"),
        Some(&object(&[("tags", JsonData::Array(vec![value("new")]))]))
    );
}

#[test]
fn metadata_field_order_and_unknown() {
    let mut updated_kv = BTreeMap::from([
        ("enable_prompt_caching".to_owned(), JsonData::Bool(true)),
        ("tags".to_owned(), JsonData::Array(Vec::new())),
        ("unknown".to_owned(), JsonData::Bool(true)),
        ("metadata".to_owned(), value("not-a-dict")),
    ]);
    let error = update_metadata_fields(&mut updated_kv).unwrap_err();
    assert_eq!(
        error,
        ManagementGuardError::MetadataNotAnObject {
            field: "tags".to_owned()
        }
    );
    assert!(!updated_kv.contains_key("tags"));
    assert_eq!(
        updated_kv.get("enable_prompt_caching"),
        Some(&JsonData::Bool(true))
    );
    assert_eq!(updated_kv.get("unknown"), Some(&JsonData::Bool(true)));
    let mut moved = BTreeMap::from([
        ("enable_prompt_caching".to_owned(), JsonData::Bool(true)),
        ("tags".to_owned(), JsonData::Array(Vec::new())),
        ("unknown".to_owned(), JsonData::Bool(true)),
    ]);
    assert_eq!(update_metadata_fields(&mut moved), Ok(()));
    assert_eq!(
        moved.get("metadata"),
        Some(&object(&[
            ("enable_prompt_caching", JsonData::Bool(true)),
            ("tags", JsonData::Array(Vec::new())),
        ]))
    );
    assert_eq!(moved.get("unknown"), Some(&JsonData::Bool(true)));
}

#[test]
fn metadata_pop_before_malformed_target() {
    let mut updated_kv = BTreeMap::from([
        ("tags".to_owned(), JsonData::Array(vec![value("new")])),
        ("metadata".to_owned(), value("bad")),
    ]);
    let error = update_metadata_field(&mut updated_kv, "tags").unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(!updated_kv.contains_key("tags"));
    assert_eq!(updated_kv.get("metadata"), Some(&value("bad")));
}

#[test]
fn native_no_license_team_budget() {
    let valid = budget_map(&[(
        "gpt-4",
        ModelBudgetConfig {
            max_budget: Some(Amount::Finite(Decimal::from(10_i32))),
            budget_duration: Some("1d"),
            tpm_limit: None,
            rpm_limit: None,
        },
    )]);
    assert_eq!(
        validate_team_model_max_budget(Some(&valid), &settings(), clocks(), &DATABASE()),
        Ok(())
    );
    let invalid = budget_map(&[(
        "gpt-4",
        config(Some(Amount::Finite(Decimal::from(-1_i32))), Some("1d")),
    )]);
    assert_eq!(
        validate_team_model_max_budget(Some(&invalid), &settings(), clocks(), &DATABASE()),
        Err(ManagementGuardError::ModelMaxBudget {
            model: "gpt-4".to_owned(),
            received: "-1".to_owned()
        })
    );
}

#[test]
fn native_no_license_metadata() {
    let mut metadata = None;
    set_object_metadata_field(&mut metadata, "tags", JsonData::Array(vec![value("team")]));
    assert_eq!(
        metadata,
        Some(BTreeMap::from([(
            "tags".to_owned(),
            JsonData::Array(vec![value("team")])
        )]))
    );
    let mut updated_kv =
        BTreeMap::from([("guardrails".to_owned(), JsonData::Array(vec![value("g")]))]);
    assert_eq!(update_metadata_field(&mut updated_kv, "guardrails"), Ok(()));
    let denial =
        check_guardrail_opt_out(&principal(Some("u1"), &[]), Some(true), None, None, "team");
    assert_eq!(
        denial,
        Err(ManagementGuardError::GuardrailsDisabled {
            entity: "team".to_owned()
        })
    );
}

#[rstest]
#[case::blank_name(
    vec![("  ", config(Some(Amount::Finite(Decimal::ONE)), Some("1d")))],
    "model_max_budget keys must be non-empty model names"
)]
#[case::missing_budget(
    vec![("gpt-4", config(None, Some("1d")))],
    "model_max_budget['gpt-4'].max_budget must be a non-negative finite number. Received: null"
)]
#[case::negative_budget(
    vec![("gpt-4", config(Some(Amount::Finite(Decimal::from(-1_i32))), Some("1d")))],
    "model_max_budget['gpt-4'].max_budget must be a non-negative finite number. Received: -1"
)]
#[case::nonfinite_budget(
    vec![("gpt-4", config(Some(Amount::Nan), Some("1d")))],
    "model_max_budget['gpt-4'].max_budget must be a non-negative finite number. Received: nan"
)]
#[case::missing_duration(
    vec![("gpt-4", config(Some(Amount::Finite(Decimal::ONE)), None))],
    "model_max_budget['gpt-4'] requires a budget_duration, e.g. '1d' or '30d'"
)]
#[case::invalid_duration(
    vec![("gpt-4", config(Some(Amount::Finite(Decimal::ONE)), Some("0d")))],
    INVALID_DURATION
)]
#[case::spaced_name(
    vec![(" gpt-4 ", config(None, None))],
    "model_max_budget[' gpt-4 '].max_budget must be a non-negative finite number. Received: null"
)]
#[case::separator_only_name(
    vec![("\u{1c}", config(Some(Amount::Finite(Decimal::ONE)), Some("1d")))],
    "model_max_budget keys must be non-empty model names"
)]
#[case::separator_prefixed_name(
    vec![("\u{1c}gpt-4", config(None, Some("1d")))],
    "model_max_budget['\u{1c}gpt-4'].max_budget must be a non-negative finite number. Received: null"
)]
fn team_model_max_budget_order(#[case] entries: Vec<NamedConfig>, #[case] message: &str) {
    let map = budget_map(&entries);
    let error =
        validate_team_model_max_budget(Some(&map), &settings(), clocks(), &DATABASE()).unwrap_err();
    assert_eq!(error.status_code(), Some(StatusCode::BAD_REQUEST));
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::zero_tpm(Some(0), None)]
#[case::zero_rpm(None, Some(0))]
#[case::both(Some(1), Some(1))]
fn team_model_max_budget_rate_limits(#[case] tpm: Option<i64>, #[case] rpm: Option<i64>) {
    let mut budget = config(Some(Amount::Finite(Decimal::ONE)), Some("1d"));
    budget.tpm_limit = tpm;
    budget.rpm_limit = rpm;
    let map = budget_map(&[("gpt-4", budget)]);
    let error =
        validate_team_model_max_budget(Some(&map), &settings(), clocks(), &DATABASE()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "model_max_budget['gpt-4'] tpm_limit/rpm_limit are not enforced on a team; set per-model rate limits on the key instead"
    );
}

#[test]
fn team_model_max_budget_empty_map() {
    let empty: BTreeMap<String, ModelBudgetConfig<'static>> = BTreeMap::new();
    assert_eq!(
        validate_team_model_max_budget(Some(&empty), &settings(), clocks(), &DATABASE()),
        Ok(())
    );
    assert_eq!(
        validate_team_model_max_budget(None, &settings(), clocks(), &DATABASE()),
        Ok(())
    );
}
