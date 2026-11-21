#![cfg(test)]

use std::collections::BTreeMap;

use axum::http::StatusCode;
use betterlite_auth::budget_primitives::window_start::DurationParser as _;
use betterlite_auth::model_budget_values::budget_config::BudgetRuntime;
use betterlite_auth::model_budget_values::candidates::BedrockCatalog;
use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::team_limits::{
    DefaultInternalUserParams, DefaultTeamParams, Settings,
};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::key_repository::table::TokenValue;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::table_ops::columns::TeamColumn;
use betterlite_db::team_rows::membership::{LiteLLMTeamMembership, MembershipBudget};
use betterlite_postgres_fixture::connection::PostgresFixture;
use chrono::{DateTime, Utc};
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use jiff::tz::TimeZoneDatabase;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

use crate::key_fields::values::Amount;
use crate::team_limits::defaults::{
    Caller, TeamParamField, TeamParamValue, default_team_param, is_available_team,
    should_auto_add_team_creator,
};
use crate::team_limits::durations::ZoneDurations;
use crate::team_limits::guard::{BudgetUpdateError, MaxBudgetGuard, budget_update_authority};
use crate::team_limits::memberships::{MembershipError, all_team_memberships};
use crate::team_limits::model_cap::{
    ModelBudgetUpdateError, existing_model_cap, model_budget_authority,
};
use crate::team_limits::write::{TeamWriteError, write_team_update};
use crate::team_request_contracts::update::UpdateTeamRequest;

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap()
}

fn decimal(text: &str) -> Decimal {
    text.parse::<Decimal>().unwrap()
}

fn amount(text: &str) -> Amount {
    Amount(decimal(text))
}

fn instant() -> DateTime<Utc> {
    DateTime::from_timestamp(1_767_225_600, 0).unwrap()
}

fn caller(role: Option<UserRole>) -> Caller {
    Caller {
        user_id: Some("caller".to_owned()),
        user_role: role,
    }
}

fn member() -> Caller {
    caller(Some(UserRole::InternalUser))
}

fn request(text: &str) -> UpdateTeamRequest {
    UpdateTeamRequest::from_value(&json(text)).unwrap()
}

fn team_params(params: DefaultTeamParams) -> Settings {
    Settings {
        default_team_params: Some(params),
        default_internal_user_params: None,
        disable_admin_auto_add: false,
    }
}

#[test]
fn default_team_params() {
    let settings = team_params(DefaultTeamParams {
        models: Some(vec!["gpt-4o".to_owned()]),
        max_budget: Some(decimal("25.5")),
        budget_duration: Some("30d".to_owned()),
        tpm_limit: Some(100),
        rpm_limit: Some(10),
        team_member_permissions: Some(vec!["/key/generate".to_owned()]),
        organization_id: Some("org-1".to_owned()),
    });
    assert_eq!(
        default_team_param(&settings, TeamParamField::Models),
        Some(TeamParamValue::Texts(vec!["gpt-4o".to_owned()]))
    );
    assert_eq!(
        default_team_param(&settings, TeamParamField::MaxBudget),
        Some(TeamParamValue::Money(amount("25.5")))
    );
    assert_eq!(
        default_team_param(&settings, TeamParamField::BudgetDuration),
        Some(TeamParamValue::Text("30d".to_owned()))
    );
    assert_eq!(
        default_team_param(&settings, TeamParamField::TpmLimit),
        Some(TeamParamValue::Integer(100))
    );
    assert_eq!(
        default_team_param(&settings, TeamParamField::RpmLimit),
        Some(TeamParamValue::Integer(10))
    );
    assert_eq!(
        default_team_param(&settings, TeamParamField::OrganizationId),
        Some(TeamParamValue::Text("org-1".to_owned()))
    );
    assert_eq!(
        default_team_param(&settings, TeamParamField::TeamMemberPermissions),
        Some(TeamParamValue::Texts(vec!["/key/generate".to_owned()]))
    );
}

#[rstest]
#[case::models(TeamParamField::Models)]
#[case::max_budget(TeamParamField::MaxBudget)]
#[case::budget_duration(TeamParamField::BudgetDuration)]
#[case::tpm_limit(TeamParamField::TpmLimit)]
#[case::rpm_limit(TeamParamField::RpmLimit)]
#[case::organization_id(TeamParamField::OrganizationId)]
#[case::team_member_permissions(TeamParamField::TeamMemberPermissions)]
fn absent_team_params_answer_none(#[case] field: TeamParamField) {
    assert_eq!(default_team_param(&Settings::default(), field), None);
}

#[test]
fn default_object_projection() {
    let settings = team_params(DefaultTeamParams {
        models: Some(vec!["openai/gpt-4o".to_owned(), "gpt-4o".to_owned()]),
        max_budget: None,
        budget_duration: None,
        tpm_limit: None,
        rpm_limit: None,
        team_member_permissions: Some(vec!["/key/update".to_owned(), "/key/generate".to_owned()]),
        organization_id: None,
    });
    assert_eq!(
        default_team_param(&settings, TeamParamField::Models),
        Some(TeamParamValue::Texts(vec![
            "openai/gpt-4o".to_owned(),
            "gpt-4o".to_owned()
        ]))
    );
    let empty = team_params(DefaultTeamParams {
        models: Some(Vec::new()),
        max_budget: None,
        budget_duration: None,
        tpm_limit: None,
        rpm_limit: None,
        team_member_permissions: None,
        organization_id: None,
    });
    assert_eq!(
        default_team_param(&empty, TeamParamField::Models),
        Some(TeamParamValue::Texts(Vec::new()))
    );
    let without_models = team_params(DefaultTeamParams::default());
    assert_eq!(
        default_team_param(&without_models, TeamParamField::Models),
        None
    );
}

#[test]
fn default_permissions_projection() {
    let settings = team_params(DefaultTeamParams {
        models: None,
        max_budget: None,
        budget_duration: None,
        tpm_limit: None,
        rpm_limit: None,
        team_member_permissions: Some(vec!["/key/update".to_owned(), "/key/generate".to_owned()]),
        organization_id: None,
    });
    assert_eq!(
        default_team_param(&settings, TeamParamField::TeamMemberPermissions),
        Some(TeamParamValue::Texts(vec![
            "/key/update".to_owned(),
            "/key/generate".to_owned()
        ]))
    );
}

#[rstest]
#[case::anonymous(None)]
#[case::proxy_admin(Some(UserRole::ProxyAdmin))]
#[case::org_admin(Some(UserRole::OrgAdmin))]
#[case::internal_user_viewer(Some(UserRole::InternalUserViewer))]
fn availability_ignores_caller(#[case] role: Option<UserRole>) {
    let settings = Settings {
        default_team_params: None,
        default_internal_user_params: Some(DefaultInternalUserParams {
            available_teams: Some(vec!["team-a".to_owned(), "team-b".to_owned()]),
        }),
        disable_admin_auto_add: false,
    };
    let caller = Caller {
        user_id: None,
        user_role: role,
    };
    assert!(is_available_team("team-a", &caller, &settings));
    assert!(!is_available_team("team-z", &caller, &settings));
    let no_list = Settings {
        default_team_params: None,
        default_internal_user_params: Some(DefaultInternalUserParams {
            available_teams: None,
        }),
        disable_admin_auto_add: false,
    };
    assert!(!is_available_team("team-a", &member(), &no_list));
    assert!(!is_available_team(
        "team-a",
        &member(),
        &Settings::default()
    ));
}

#[test]
fn creator_missing_id() {
    let admin = caller(Some(UserRole::ProxyAdmin));
    let anonymous = Caller {
        user_id: None,
        user_role: Some(UserRole::InternalUser),
    };
    assert!(!should_auto_add_team_creator(
        &anonymous,
        &Settings::default()
    ));
    assert!(!should_auto_add_team_creator(
        &Caller {
            user_id: None,
            user_role: admin.user_role,
        },
        &Settings::default()
    ));
}

#[test]
fn creator_empty_id() {
    let empty = Caller {
        user_id: Some(String::new()),
        user_role: Some(UserRole::InternalUser),
    };
    assert!(should_auto_add_team_creator(&empty, &Settings::default()));
    assert!(should_auto_add_team_creator(
        &member(),
        &Settings::default()
    ));
}

#[test]
fn creator_auto_add_disabled() {
    let disabled = Settings {
        default_team_params: None,
        default_internal_user_params: None,
        disable_admin_auto_add: true,
    };
    let admin = caller(Some(UserRole::ProxyAdmin));
    assert!(!should_auto_add_team_creator(&admin, &disabled));
    assert!(should_auto_add_team_creator(&admin, &Settings::default()));
    assert!(should_auto_add_team_creator(&member(), &disabled));
}

#[test]
fn team_admin_budget_escalation() {
    let error = budget_update_authority(
        &request(r#"{"team_id":"t","max_budget":101}"#),
        &member(),
        Some(amount("100")),
    )
    .unwrap_err();
    assert_eq!(
        error,
        BudgetUpdateError::RaiseDenied {
            existing: decimal("100"),
            requested: decimal("101"),
        }
    );
    assert_eq!(
        error.to_string(),
        "Only a proxy admin can raise a team's max_budget. Team's current max_budget=100, requested=101."
    );
    assert_eq!(
        budget_update_authority(
            &request(r#"{"team_id":"t","max_budget":10}"#),
            &member(),
            None,
        ),
        Ok(Some(MaxBudgetGuard { expected: None }))
    );
}

#[rstest]
#[case::raise(r#"{"team_id":"t","max_budget":200}"#)]
#[case::remove(r#"{"team_id":"t","max_budget":null}"#)]
#[case::omit(r#"{"team_id":"t","team_alias":"a"}"#)]
fn proxy_budget_bypass(#[case] text: &str) {
    let admin = caller(Some(UserRole::ProxyAdmin));
    assert_eq!(
        budget_update_authority(&request(text), &admin, Some(amount("100"))),
        Ok(None)
    );
}

#[test]
fn omitted_budget_no_guard() {
    let text = r#"{"team_id":"t","team_alias":"a"}"#;
    assert_eq!(
        budget_update_authority(&request(text), &member(), Some(amount("100"))),
        Ok(None)
    );
    assert_eq!(
        budget_update_authority(&request(text), &member(), None),
        Ok(None)
    );
}

#[rstest]
#[case::positive("10")]
#[case::zero("0")]
#[case::large("1000")]
fn uncapped_budget_guard(#[case] value: &str) {
    let text = format!(r#"{{"team_id":"t","max_budget":{value}}}"#);
    assert_eq!(
        budget_update_authority(&request(&text), &member(), None),
        Ok(Some(MaxBudgetGuard { expected: None }))
    );
}

#[test]
fn cap_null_denied() {
    let error = budget_update_authority(
        &request(r#"{"team_id":"t","max_budget":null}"#),
        &member(),
        Some(amount("100")),
    )
    .unwrap_err();
    assert_eq!(
        error,
        BudgetUpdateError::RemoveDenied {
            existing: decimal("100"),
        }
    );
    assert_eq!(
        error.to_string(),
        "Only a proxy admin can remove a team's max_budget. Team's current max_budget=100."
    );
}

#[test]
fn cap_lower_allowed() {
    assert_eq!(
        budget_update_authority(
            &request(r#"{"team_id":"t","max_budget":50}"#),
            &member(),
            Some(amount("100")),
        ),
        Ok(Some(MaxBudgetGuard {
            expected: Some(amount("100")),
        }))
    );
}

#[test]
fn cap_equal_allowed() {
    assert_eq!(
        budget_update_authority(
            &request(r#"{"team_id":"t","max_budget":100}"#),
            &member(),
            Some(amount("100")),
        ),
        Ok(Some(MaxBudgetGuard {
            expected: Some(amount("100")),
        }))
    );
}

fn caps(text: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(text).unwrap()
}

struct NoBedrock;

impl BedrockCatalog for NoBedrock {
    fn base_model(&self, model: &str) -> String {
        model.to_owned()
    }

    fn prices_as_bedrock(&self, _base_model: &str) -> bool {
        false
    }
}

struct Bedrock;

impl BedrockCatalog for Bedrock {
    fn base_model(&self, model: &str) -> String {
        model.strip_prefix("us.").unwrap_or(model).to_owned()
    }

    fn prices_as_bedrock(&self, base_model: &str) -> bool {
        base_model == "anthropic.claude"
    }
}

fn check_models(
    role: Option<UserRole>,
    text: &str,
    existing: &BTreeMap<String, JsonData>,
    catalog: &dyn BedrockCatalog,
) -> Result<(), ModelBudgetUpdateError> {
    let zones = TimeZoneDatabase::from_env();
    let durations = ZoneDurations { database: &zones };
    model_budget_authority(
        &request(text),
        &caller(role),
        Some(existing),
        catalog,
        BudgetRuntime {
            durations: &durations,
            now: instant(),
        },
    )
}

fn model_verdict(text: &str, existing: &str) -> Result<(), ModelBudgetUpdateError> {
    check_models(
        Some(UserRole::InternalUser),
        text,
        &caps(existing),
        &NoBedrock,
    )
}

#[test]
fn malformed_model_cap() {
    let existing = caps(r#"{"m":{"max_budget":"not-a-number","budget_duration":"1d"}}"#);
    assert_eq!(existing_model_cap(&existing["m"]), None);
    let without_duration = caps(r#"{"m":{"max_budget":10.0}}"#);
    assert!(existing_model_cap(&without_duration["m"]).is_some());
    assert_eq!(
        model_verdict(
            r#"{"team_id":"t","model_max_budget":{}}"#,
            r#"{"m":{"max_budget":"not-a-number","budget_duration":"1d"}}"#
        ),
        Ok(())
    );
}

#[test]
fn omitted_models_allowed() {
    assert_eq!(
        model_verdict(
            r#"{"team_id":"t","team_alias":"a"}"#,
            r#"{"m":{"max_budget":10.0,"budget_duration":"1d"}}"#
        ),
        Ok(())
    );
}

#[rstest]
#[case::empty_map(r#"{"team_id":"t","model_max_budget":{}}"#)]
#[case::null(r#"{"team_id":"t","model_max_budget":null}"#)]
fn removed_model_cap_denied(#[case] text: &str) {
    let existing = r#"{"m":{"max_budget":10.0,"budget_duration":"1d"}}"#;
    let error = model_verdict(text, existing).unwrap_err();
    assert_eq!(
        error,
        ModelBudgetUpdateError::RemoveDenied {
            model: "m".to_owned(),
            max_budget: decimal("10"),
        }
    );
    assert!(error.to_string().contains("'m'"));
    assert_eq!(
        check_models(
            Some(UserRole::ProxyAdmin),
            r#"{"team_id":"t","model_max_budget":{}}"#,
            &caps(existing),
            &NoBedrock
        ),
        Ok(())
    );
}

#[test]
fn wildcard_cap_denied() {
    let existing = r#"{"anthropic.claude":{"max_budget":10.0,"budget_duration":"1d"}}"#;
    let text = r#"{"team_id":"t","model_max_budget":{"anthropic.claude":{"max_budget":10.0,"budget_duration":"1d"},"us.anthropic.claude":{"max_budget":20.0,"budget_duration":"1d"}}}"#;
    let error = check_models(
        Some(UserRole::InternalUser),
        text,
        &caps(existing),
        &Bedrock,
    )
    .unwrap_err();
    let ModelBudgetUpdateError::RaiseDenied {
        model,
        max_budget,
        entry,
        ..
    } = error
    else {
        panic!("the bare-family entry governs the raised wildcard model");
    };
    assert_eq!(model, "us.anthropic.claude");
    assert_eq!(entry, "anthropic.claude");
    assert_eq!(max_budget, decimal("10"));
}

#[test]
fn model_window_change_denied() {
    let error = model_verdict(
        r#"{"team_id":"t","model_max_budget":{"m":{"max_budget":5.0,"budget_duration":"2d"}}}"#,
        r#"{"m":{"max_budget":10.0,"budget_duration":"1d"}}"#,
    )
    .unwrap_err();
    assert!(error.to_string().contains("requested=5.0 per 2d"));
}

#[test]
fn model_lower_same_window() {
    assert_eq!(
        model_verdict(
            r#"{"team_id":"t","model_max_budget":{"m":{"max_budget":5.0,"budget_duration":"1d"}}}"#,
            r#"{"m":{"max_budget":10.0,"budget_duration":"1d"}}"#
        ),
        Ok(())
    );
}

#[test]
fn model_equal_same_window() {
    assert_eq!(
        model_verdict(
            r#"{"team_id":"t","model_max_budget":{"m":{"max_budget":10.0,"budget_duration":"1d"}}}"#,
            r#"{"m":{"max_budget":10.0,"budget_duration":"1d"}}"#
        ),
        Ok(())
    );
}

#[test]
fn duration_parser_reads_budget_specs() {
    let zones = TimeZoneDatabase::from_env();
    let durations = ZoneDurations { database: &zones };
    assert_eq!(durations.seconds("1d", instant()), Some(86_400));
    assert_eq!(durations.seconds("90m", instant()), Some(5_400));
    assert_eq!(durations.seconds("not-a-duration", instant()), None);
}

#[test]
fn uncapped_model_allowed() {
    assert_eq!(
        model_verdict(
            r#"{"team_id":"t","model_max_budget":{"m":{"max_budget":50.0,"budget_duration":"1d"}}}"#,
            r#"{"m":{"budget_duration":"1d"}}"#
        ),
        Ok(())
    );
    assert_eq!(
        model_verdict(
            r#"{"team_id":"t","model_max_budget":{"m":{"max_budget":10.0,"budget_duration":"1d"},"other":{"max_budget":99.0,"budget_duration":"1d"}}}"#,
            r#"{"m":{"max_budget":10.0,"budget_duration":"1d"}}"#
        ),
        Ok(())
    );
}

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

const SCHEMA: &str = r#"
CREATE TABLE "LiteLLM_TeamTable" (
    team_id TEXT PRIMARY KEY,
    team_alias TEXT,
    models TEXT[] NOT NULL DEFAULT '{}',
    access_group_ids TEXT[],
    blocked BOOLEAN NOT NULL DEFAULT false,
    metadata JSONB,
    tpm_limit BIGINT,
    max_budget DOUBLE PRECISION,
    model_spend JSONB,
    model_max_budget JSONB,
    model_id BIGINT,
    object_permission_id TEXT,
    updated_at TIMESTAMP
);
CREATE TABLE "LiteLLM_ModelTable" (
    id BIGINT PRIMARY KEY,
    model_aliases JSONB,
    created_by TEXT NOT NULL,
    updated_by TEXT NOT NULL
);
CREATE TABLE "LiteLLM_ObjectPermissionTable" (
    object_permission_id TEXT PRIMARY KEY
);
CREATE TABLE "LiteLLM_TeamMembership" (
    user_id TEXT NOT NULL,
    team_id TEXT NOT NULL,
    budget_id TEXT,
    spend DOUBLE PRECISION NOT NULL DEFAULT 0,
    total_spend DOUBLE PRECISION NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, team_id)
);
CREATE TABLE "LiteLLM_BudgetTable" (
    budget_id TEXT PRIMARY KEY,
    max_budget DOUBLE PRECISION
);
INSERT INTO "LiteLLM_ModelTable" (id, created_by, updated_by) VALUES (7, 'seed', 'seed');
INSERT INTO "LiteLLM_ObjectPermissionTable" (object_permission_id) VALUES ('op-1');
INSERT INTO "LiteLLM_TeamTable" (team_id, max_budget, model_id, object_permission_id) VALUES
    ('team-open', NULL, NULL, NULL),
    ('team-capped', 100, NULL, NULL),
    ('team-stale', 100, NULL, NULL),
    ('team-broken', 10, NULL, NULL),
    ('team-linked', 20, 7, 'op-1');
UPDATE "LiteLLM_TeamTable" SET model_spend = '"broken"'::jsonb WHERE team_id = 'team-broken';
INSERT INTO "LiteLLM_BudgetTable" (budget_id, max_budget) VALUES ('b-1', 5);
INSERT INTO "LiteLLM_TeamMembership" (user_id, team_id, budget_id, spend) VALUES
    ('u1', 'team-open', NULL, 2.5),
    ('u2', 'team-open', 'b-1', 1.5),
    ('u1', 'team-capped', NULL, 0),
    ('', 'team-open', NULL, 0);
"#;

async fn database() -> (PostgresFixture, Pool) {
    let url = Environment::parse_from(["team-limits-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_with_statements(url.as_ref(), vec![SCHEMA.to_owned()])
        .resolve()
        .await
        .unwrap();
    let manager = Manager::new(fixture.options.url.expose_secret().parse().unwrap(), NoTls);
    let pool = Pool::builder(manager).max_size(1).build().unwrap();
    (fixture, pool)
}

fn store(pool: &Pool) -> PostgresStore {
    PostgresStore {
        pool: Some(pool.clone()),
    }
}

async fn close(fixture: PostgresFixture, pool: Pool) {
    pool.close();
    fixture.close_async().await.unwrap();
}

fn update(fields: &[(TeamColumn, TokenValue)]) -> BTreeMap<TeamColumn, TokenValue> {
    fields.iter().cloned().collect()
}

async fn stored_alias(pool: &Pool, team_id: &str) -> Option<String> {
    let client = pool.get().await.unwrap();
    let row = client
        .query_one(
            "SELECT team_alias FROM \"LiteLLM_TeamTable\" WHERE team_id = $1",
            &[&team_id],
        )
        .await
        .unwrap();
    row.get(0)
}

#[tokio::test]
async fn postgres_guarded_update() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let guard = MaxBudgetGuard {
        expected: Some(amount("100")),
    };
    let data = update(&[
        (
            TeamColumn::TeamAlias,
            TokenValue::Text("renamed".to_owned()),
        ),
        (
            TeamColumn::Models,
            TokenValue::TextArray(vec!["gpt-4o".to_owned()]),
        ),
        (TeamColumn::Blocked, TokenValue::Bool(true)),
        (TeamColumn::Metadata, TokenValue::Json(json(r#"{"k":1}"#))),
        (TeamColumn::TpmLimit, TokenValue::Integer(25)),
        (TeamColumn::MaxBudget, TokenValue::Number(decimal("90"))),
    ]);
    let written = write_team_update(&store, "team-stale", &data, Some(&guard), instant())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(written.fields.base.team_alias.as_deref(), Some("renamed"));
    assert_eq!(written.fields.base.models, vec!["gpt-4o".to_owned()]);
    assert!(written.fields.base.blocked);
    assert_eq!(written.fields.base.max_budget, Some(decimal("90")));
    assert_eq!(
        written.fields.updated_at,
        Some(Timestamp::Naive(instant().naive_utc()))
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_unguarded_clear() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let earlier = Timestamp::Naive(
        DateTime::from_timestamp(1_700_000_000, 0)
            .unwrap()
            .naive_utc(),
    );
    let cleared = update(&[
        (TeamColumn::TeamAlias, TokenValue::Null),
        (TeamColumn::AccessGroupIds, TokenValue::Null),
        (TeamColumn::UpdatedAt, TokenValue::Timestamp(earlier)),
    ]);
    let after = write_team_update(&store, "team-stale", &cleared, None, instant())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.fields.base.team_alias, None);
    assert_eq!(after.fields.base.access_group_ids, None);
    assert_eq!(after.fields.updated_at, Some(earlier));
    close(fixture, pool).await;
}

#[tokio::test]
async fn stale_cap_409() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let verdict = budget_update_authority(
        &request(r#"{"team_id":"team-stale","max_budget":100}"#),
        &member(),
        Some(amount("100")),
    )
    .unwrap()
    .unwrap();
    let client = pool.get().await.unwrap();
    client
        .execute(
            "UPDATE \"LiteLLM_TeamTable\" SET max_budget = 50 WHERE team_id = 'team-stale'",
            &[],
        )
        .await
        .unwrap();
    drop(client);
    let data = update(&[(TeamColumn::TeamAlias, TokenValue::Text("late".to_owned()))]);
    let error = write_team_update(&store, "team-stale", &data, Some(&verdict), instant())
        .await
        .unwrap_err();
    assert_eq!(error.status_code(), StatusCode::CONFLICT);
    assert_eq!(
        error.to_string(),
        "The team's max_budget changed during this update. Reload the team and try again."
    );
    assert_eq!(stored_alias(&pool, "team-stale").await, None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn null_cap_compare() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let data = update(&[(TeamColumn::TeamAlias, TokenValue::Text("first".to_owned()))]);
    let uncapped = MaxBudgetGuard { expected: None };
    assert!(
        write_team_update(&store, "team-open", &data, Some(&uncapped), instant())
            .await
            .unwrap()
            .is_some()
    );
    let stale_null = write_team_update(&store, "team-capped", &data, Some(&uncapped), instant())
        .await
        .unwrap_err();
    assert_eq!(stale_null.status_code(), StatusCode::CONFLICT);
    let stale_value = MaxBudgetGuard {
        expected: Some(amount("10")),
    };
    let against_null = write_team_update(&store, "team-open", &data, Some(&stale_value), instant())
        .await
        .unwrap_err();
    assert_eq!(against_null.status_code(), StatusCode::CONFLICT);
    assert_eq!(stored_alias(&pool, "team-capped").await, None);
    assert_eq!(
        stored_alias(&pool, "team-open").await.as_deref(),
        Some("first")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn guarded_read_failure() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let guard = MaxBudgetGuard {
        expected: Some(amount("10")),
    };
    let data = update(&[(
        TeamColumn::TeamAlias,
        TokenValue::Text("committed".to_owned()),
    )]);
    let error = write_team_update(&store, "team-broken", &data, Some(&guard), instant())
        .await
        .unwrap_err();
    assert!(matches!(error, TeamWriteError::Decode(_)));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        stored_alias(&pool, "team-broken").await.as_deref(),
        Some("committed")
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_missing_team() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let data = update(&[(TeamColumn::TeamAlias, TokenValue::Text("ghost".to_owned()))]);
    let written = write_team_update(&store, "team-missing", &data, None, instant())
        .await
        .unwrap();
    assert_eq!(written, None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn update_includes_permissions() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let data = update(&[(TeamColumn::TeamAlias, TokenValue::Text("linked".to_owned()))]);
    let written = write_team_update(&store, "team-linked", &data, None, instant())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        written
            .fields
            .object_permission
            .as_ref()
            .map(|permission| permission.object_permission_id.as_str()),
        Some("op-1")
    );
    assert_eq!(
        written
            .fields
            .litellm_model_table
            .as_ref()
            .map(|table| table.created_by.as_str()),
        Some("seed")
    );
    close(fixture, pool).await;
}

fn team_ids(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| String::from(*text)).collect()
}

fn membership_ids(memberships: &[LiteLLMTeamMembership]) -> Vec<String> {
    memberships
        .iter()
        .map(|membership| format!("{}:{}", membership.team_id, membership.user_id))
        .collect()
}

#[tokio::test]
async fn memberships_user_filter() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let teams = team_ids(&["team-open", "team-capped"]);
    let all = all_team_memberships(&store, &teams, None).await.unwrap();
    assert_eq!(
        membership_ids(&all),
        team_ids(&[
            "team-capped:u1",
            "team-open:",
            "team-open:u1",
            "team-open:u2",
        ])
    );
    let one = all_team_memberships(&store, &teams, Some("u1"))
        .await
        .unwrap();
    assert_eq!(
        membership_ids(&one),
        team_ids(&["team-capped:u1", "team-open:u1"])
    );
    let empty = all_team_memberships(&store, &teams, Some(""))
        .await
        .unwrap();
    assert_eq!(membership_ids(&empty), team_ids(&["team-open:"]));
    assert!(
        all_team_memberships(&store, &[], None)
            .await
            .unwrap()
            .is_empty()
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn memberships_include_budget() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let teams = team_ids(&["team-open"]);
    let with_budget = all_team_memberships(&store, &teams, Some("u2"))
        .await
        .unwrap();
    let membership = with_budget.first().unwrap();
    assert_eq!(membership.budget_id.as_deref(), Some("b-1"));
    let budget = membership.litellm_budget_table.as_ref().unwrap();
    let MembershipBudget::Base(row) = budget else {
        panic!("the budget row has no server-only timestamps");
    };
    assert_eq!(row.budget_id.as_deref(), Some("b-1"));
    assert_eq!(row.max_budget, Some(decimal("5")));
    let without_budget = all_team_memberships(&store, &teams, Some("u1"))
        .await
        .unwrap();
    assert_eq!(without_budget.first().unwrap().litellm_budget_table, None);
    close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_memberships() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let teams = team_ids(&["team-open"]);
    let members = all_team_memberships(&store, &teams, None).await.unwrap();
    assert_eq!(
        membership_ids(&members),
        team_ids(&["team-open:", "team-open:u1", "team-open:u2"])
    );
    let spent = members
        .iter()
        .find(|membership| membership.user_id == "u2")
        .unwrap();
    assert_eq!(spent.spend.0, Some(decimal("1.5")));
    assert_eq!(spent.total_spend.0, Some(decimal("0")));
    let filtered = all_team_memberships(&store, &teams, Some("u2"))
        .await
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered.first().unwrap().budget_id.as_deref(), Some("b-1"));
    assert!(
        all_team_memberships(&store, &team_ids(&["other"]), None)
            .await
            .unwrap()
            .is_empty()
    );
    close(fixture, pool).await;
}

#[tokio::test]
async fn postgres_reread_failure() {
    let (fixture, pool) = database().await;
    let store = store(&pool);
    let guard = MaxBudgetGuard {
        expected: Some(amount("10")),
    };
    let data = update(&[
        (TeamColumn::TeamAlias, TokenValue::Text("kept".to_owned())),
        (TeamColumn::MaxBudget, TokenValue::Number(decimal("8"))),
    ]);
    let error = write_team_update(&store, "team-broken", &data, Some(&guard), instant())
        .await
        .unwrap_err();
    assert!(matches!(error, TeamWriteError::Decode(_)));
    let client = pool.get().await.unwrap();
    let row = client
        .query_one(
            "SELECT team_alias, max_budget::text FROM \"LiteLLM_TeamTable\" WHERE team_id = 'team-broken'",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<String>>(0).as_deref(), Some("kept"));
    assert_eq!(row.get::<_, Option<String>>(1).as_deref(), Some("8"));
    drop(client);
    assert!(matches!(
        all_team_memberships(&PostgresStore { pool: None }, &team_ids(&["a"]), None)
            .await
            .unwrap_err(),
        MembershipError::Database(_)
    ));
    close(fixture, pool).await;
}
