#![cfg(test)]

mod postgres;
mod team;

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable, Timestamp};
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::auth_end_users::defaults::MemberBudgetError;
use crate::auth_key_budgets::alerts::{BudgetAlert, BudgetAlertKind, BudgetAlertSink, EventGroup};
use crate::auth_key_budgets::key_budget::Denial;
use crate::auth_key_budgets::spend::{SpendReadError, SpendReader};
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::denial::model_denied_client_message;
use crate::auth_model_patterns::pattern::UnresolvedProviderInference;
use crate::auth_request_models::RequestedModels;
use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window_start::DurationParser;
use crate::current_spend::SpendQuery;
use crate::membership_load::error::MembershipLoadError;
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

use super::alerts::{TeamAlert, TeamAlertSink};
use super::error::TeamBudgetError;
use super::soft::SoftOutcome;
use super::sources::{DefaultBudgetSource, Membership, MembershipSource};
use super::{TeamAdmission, TeamRequest, admit};

const MEMBER_KEY: &str = r#"{"token":"sk-member","user_id":"user-1","team_id":"team-1","team_alias":"Alpha","org_id":"org-1","key_alias":"member-key"}"#;
pub(super) const DEFAULT_DEPTH: i64 = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Read {
    pub(super) counter_key: String,
    pub(super) fallback: Decimal,
    pub(super) max_budget: Option<Decimal>,
    pub(super) entity_type: Option<CounterEntityType>,
    pub(super) entity_id: Option<String>,
    pub(super) duration: Option<String>,
    pub(super) start: Option<Timestamp>,
}

// every collaborator at once, so one log shows the order of reads and alerts
#[derive(Default)]
pub(super) struct Harness {
    pub(super) spends: Mutex<Vec<Decimal>>,
    pub(super) reads: Mutex<Vec<Read>>,
    pub(super) stored_membership: Option<LiteLLMTeamMembership>,
    pub(super) membership_fails: bool,
    pub(super) membership_queries: AtomicUsize,
    pub(super) defaults: BTreeMap<String, LiteLLMBudgetTable>,
    pub(super) default_queries: Mutex<Vec<String>>,
    pub(super) member_alerts: Mutex<Vec<BudgetAlert>>,
    pub(super) team_alerts: Mutex<Vec<TeamAlert>>,
    pub(super) events: Mutex<Vec<String>>,
}

impl Harness {
    pub(super) fn spending(spends: &[&str]) -> Self {
        let harness = Self::default();
        *harness.spends.lock().unwrap() = spends.iter().copied().map(money).collect();
        harness
    }

    fn with_default(mut self, budget_id: &str, max_budget: &str) -> Self {
        self.defaults
            .insert(budget_id.to_owned(), budget(max_budget));
        self
    }

    fn failing(mut self) -> Self {
        self.membership_fails = true;
        self
    }

    pub(super) fn reads(&self) -> Vec<Read> {
        self.reads.lock().unwrap().clone()
    }

    pub(super) fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    pub(super) fn team_alerts(&self) -> Vec<TeamAlert> {
        self.team_alerts.lock().unwrap().clone()
    }
}

#[async_trait]
impl SpendReader for Harness {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("read {}", query.counter_key));
        self.reads.lock().unwrap().push(Read {
            counter_key: query.counter_key.to_owned(),
            fallback: query.fallback_spend,
            max_budget: query.max_budget,
            entity_type: query.window.entity_type.cloned(),
            entity_id: query.window.entity_id.map(str::to_owned),
            duration: query.window.duration.map(str::to_owned),
            start: query.window.start,
        });
        let mut spends = self.spends.lock().unwrap();
        assert!(!spends.is_empty(), "the harness ran out of scripted spends");
        Ok(spends.remove(0))
    }
}

#[async_trait]
impl MembershipSource for Harness {
    async fn team_membership(
        &self,
        _user_id: &str,
        _team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.membership_queries.fetch_add(1, Ordering::SeqCst);
        if self.membership_fails {
            return Err(MembershipLoadError::NoDatabase);
        }
        Ok(self.stored_membership.clone())
    }
}

#[async_trait]
impl DefaultBudgetSource for Harness {
    async fn team_member_default(
        &self,
        budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        self.default_queries
            .lock()
            .unwrap()
            .push(budget_id.to_owned());
        Ok(self.defaults.get(budget_id).cloned())
    }
}

impl BudgetAlertSink for Harness {
    fn schedule(&self, alert: BudgetAlert) {
        self.events.lock().unwrap().push("member alert".to_owned());
        self.member_alerts.lock().unwrap().push(alert);
    }
}

impl TeamAlertSink for Harness {
    fn schedule(&self, alert: TeamAlert) {
        self.events
            .lock()
            .unwrap()
            .push(format!("team alert {:?}", alert.kind));
        self.team_alerts.lock().unwrap().push(alert);
    }
}

impl DurationParser for Harness {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        match spec {
            "1h" => Some(3_600_i64),
            "1d" => Some(86_400_i64),
            _ => None,
        }
    }
}

pub(super) fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
}

pub(super) fn money(text: &str) -> Decimal {
    text.parse().unwrap()
}

pub(super) fn team(fields: &str) -> LiteLLMTeamTable {
    serde_json::from_str(fields).unwrap()
}

pub(super) fn member(fields: &str) -> LiteLLMTeamMembership {
    serde_json::from_str(fields).unwrap()
}

pub(super) fn key(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = serde_json::from_str(fields).unwrap() else {
        panic!("the key fields must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).unwrap()
}

pub(super) fn request<'request>(
    team: Option<&'request LiteLLMTeamTable>,
    token: Option<&'request UserAPIKeyAuth>,
    model: &'request RequestedModels,
    membership: Membership,
) -> TeamRequest<'request> {
    TeamRequest {
        team,
        token,
        user_email: Some("member@example.com"),
        model,
        membership,
    }
}

pub(super) async fn admit_with(
    harness: &Harness,
    memberships: &dyn MembershipSource,
    defaults: &dyn DefaultBudgetSource,
    request: &TeamRequest<'_>,
    max_recurse_depth: i64,
) -> Result<SoftOutcome, TeamBudgetError> {
    let aliases = BTreeMap::new();
    let provider = UnresolvedProviderInference;
    let models = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth,
    };
    let admission = TeamAdmission {
        reader: harness,
        memberships,
        defaults,
        member_alerts: Some(harness),
        team_alerts: Some(harness),
        durations: harness,
        models: &models,
        now: now(),
    };
    admit(&admission, request).await
}

async fn admit_at(
    harness: &Harness,
    request: &TeamRequest<'_>,
    max_recurse_depth: i64,
) -> Result<SoftOutcome, TeamBudgetError> {
    admit_with(harness, harness, harness, request, max_recurse_depth).await
}

pub(super) async fn run(
    harness: &Harness,
    request: &TeamRequest<'_>,
) -> Result<SoftOutcome, TeamBudgetError> {
    admit_at(harness, request, DEFAULT_DEPTH).await
}

fn loaded(fields: &str) -> Membership {
    Membership::Loaded(Some(Box::new(member(fields))))
}

fn denial_of(result: Result<SoftOutcome, TeamBudgetError>) -> Denial {
    match result {
        Err(TeamBudgetError::Denied(denial)) => denial,
        other => panic!("expected a budget denial, got {other:?}"),
    }
}

fn budget(max_budget: &str) -> LiteLLMBudgetTable {
    serde_json::from_str(&format!(r#"{{"max_budget":{max_budget}}}"#)).unwrap()
}

#[rstest]
#[case::no_team(false, MEMBER_KEY)]
#[case::no_token(true, "")]
#[case::no_user(true, r#"{"token":"sk-anonymous","team_id":"team-1"}"#)]
#[tokio::test]
async fn missing_identity_skips(#[case] with_team: bool, #[case] key_fields: &str) {
    let harness = Harness::default();
    let row = team(r#"{"team_id":"team-1","metadata":{"team_member_budget_id":"default-1"}}"#);
    let token = (!key_fields.is_empty()).then(|| key(key_fields));
    let model = RequestedModels::One("gpt-4o".to_owned());
    let checked = request(
        with_team.then_some(&row),
        token.as_ref(),
        &model,
        Membership::Unloaded,
    );
    assert_eq!(
        run(&harness, &checked).await.unwrap(),
        SoftOutcome::NotCrossed
    );
    assert_eq!(harness.membership_queries.load(Ordering::SeqCst), 0);
    assert!(harness.default_queries.lock().unwrap().is_empty());
    assert!(harness.reads().is_empty());
}

#[tokio::test]
async fn loaded_none_avoids_query() {
    let harness = Harness::default();
    let row = team(r#"{"team_id":"team-1"}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::One("gpt-4o".to_owned());
    let preloaded = request(Some(&row), Some(&token), &model, Membership::Loaded(None));
    run(&harness, &preloaded).await.unwrap();
    assert_eq!(harness.membership_queries.load(Ordering::SeqCst), 0);
    // an unloaded membership is read once and shared by the scope and budget checks
    let unloaded = request(Some(&row), Some(&token), &model, Membership::Unloaded);
    run(&harness, &unloaded).await.unwrap();
    assert_eq!(harness.membership_queries.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn member_override_zero_denies() {
    let harness = Harness::spending(&["0"]);
    let row = team(r#"{"team_id":"team-1","metadata":{"team_member_budget_id":"default-1"}}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    let membership = loaded(
        r#"{"user_id":"user-1","team_id":"team-1","spend":0,"litellm_budget_table":{"max_budget":0}}"#,
    );
    let denial = denial_of(
        run(
            &harness,
            &request(Some(&row), Some(&token), &model, membership),
        )
        .await,
    );
    assert_eq!(denial.max_budget, Decimal::ZERO);
    assert_eq!(denial.current_cost, Decimal::ZERO);
    // the override wins, so the team default is never looked up
    assert!(harness.default_queries.lock().unwrap().is_empty());
}

#[rstest]
#[case::positive_default("7", Some("7"))]
#[case::zero_default("0", None)]
#[case::negative_default("-1", None)]
#[tokio::test]
async fn positive_default_only(#[case] default_max: &str, #[case] cap: Option<&str>) {
    let harness = Harness::spending(&["1"]).with_default("default-1", default_max);
    let row = team(r#"{"team_id":"team-1","metadata":{"team_member_budget_id":"default-1"}}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    // a member row without its own max_budget falls back to the default
    let membership = loaded(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"max_budget":null}}"#,
    );
    run(
        &harness,
        &request(Some(&row), Some(&token), &model, membership),
    )
    .await
    .unwrap();
    assert_eq!(
        *harness.default_queries.lock().unwrap(),
        vec!["default-1".to_owned()]
    );
    let caps: Vec<Option<Decimal>> = harness.reads().iter().map(|read| read.max_budget).collect();
    let expected: Vec<Option<Decimal>> = cap.into_iter().map(|value| Some(money(value))).collect();
    assert_eq!(caps, expected);
}

#[rstest]
#[case::missing_default_row(r#"{"team_member_budget_id":"absent"}"#, 1)]
#[case::numeric_default_id(r#"{"team_member_budget_id":7}"#, 0)]
#[case::no_default_id("{}", 0)]
#[tokio::test]
async fn positive_default_only_skips_without_a_default(
    #[case] metadata: &str,
    #[case] lookups: usize,
) {
    let harness = Harness::default();
    let row = team(&format!(r#"{{"team_id":"team-1","metadata":{metadata}}}"#));
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    run(
        &harness,
        &request(Some(&row), Some(&token), &model, Membership::Loaded(None)),
    )
    .await
    .unwrap();
    assert_eq!(harness.default_queries.lock().unwrap().len(), lookups);
    assert!(harness.reads().is_empty());
}

#[rstest]
#[case::override_expires_now("10", 0, "10")]
#[case::override_active("10", 1, "15")]
#[case::default_expires_now("null", 0, "8")]
#[case::default_active("null", 1, "13")]
#[tokio::test]
async fn temporary_increase_expiry_boundary(
    #[case] member_max: &str,
    #[case] seconds_after_now: i64,
    #[case] cap: &str,
) {
    let harness = Harness::spending(&["0"]).with_default("default-1", "8");
    let row = team(r#"{"team_id":"team-1","metadata":{"team_member_budget_id":"default-1"}}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    let expiry = now()
        .checked_add_signed(TimeDelta::seconds(seconds_after_now))
        .unwrap()
        .to_rfc3339();
    let membership = loaded(&format!(
        r#"{{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{{"max_budget":{member_max},"temp_budget_increase":5,"temp_budget_expiry":"{expiry}"}}}}"#
    ));
    run(
        &harness,
        &request(Some(&row), Some(&token), &model, membership),
    )
    .await
    .unwrap();
    assert_eq!(harness.reads()[0].max_budget, Some(money(cap)));
}

#[rstest]
#[case::row_spend(r#""spend":3.5,"#, "3.5")]
#[case::null_spend(r#""spend":null,"#, "0")]
#[tokio::test]
async fn member_counter_key_and_fallback(#[case] spend_field: &str, #[case] fallback: &str) {
    let harness = Harness::spending(&["0"]);
    let row = team(r#"{"team_id":"team-1"}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    let membership = loaded(&format!(
        r#"{{"user_id":"user-1","team_id":"team-1",{spend_field}"litellm_budget_table":{{"max_budget":20}}}}"#
    ));
    run(
        &harness,
        &request(Some(&row), Some(&token), &model, membership),
    )
    .await
    .unwrap();
    assert_eq!(
        harness.reads(),
        vec![Read {
            counter_key: "spend:team_member:user-1:team-1".to_owned(),
            fallback: money(fallback),
            max_budget: Some(money("20")),
            entity_type: None,
            entity_id: None,
            duration: None,
            start: None,
        }]
    );
}

#[rstest]
#[case::below("19.99", false)]
#[case::equal("20", true)]
#[tokio::test]
async fn alert_precedes_member_equality_denial(#[case] spend: &str, #[case] denied: bool) {
    let harness = Harness::spending(&[spend]);
    let row = team(
        r#"{"team_id":"team-1","metadata":{"team_member_max_budget_alert_emails":{"50":["lead@example.com"]}}}"#,
    );
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    let membership = loaded(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"max_budget":20}}"#,
    );
    let result = run(
        &harness,
        &request(Some(&row), Some(&token), &model, membership),
    )
    .await;
    assert_eq!(matches!(result, Err(TeamBudgetError::Denied(_))), denied);
    assert_eq!(
        harness.events(),
        vec![
            "read spend:team_member:user-1:team-1".to_owned(),
            "member alert".to_owned()
        ]
    );
}

#[tokio::test]
async fn member_entity_and_email() {
    let harness = Harness::spending(&["30"]);
    let row = team(
        r#"{"team_id":"team-1","team_alias":"Alpha team","organization_id":"org-team","metadata":{"team_member_max_budget_alert_emails":{"100":["lead@example.com"]}}}"#,
    );
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    let membership = loaded(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"max_budget":20}}"#,
    );
    let denial = denial_of(
        run(
            &harness,
            &request(Some(&row), Some(&token), &model, membership),
        )
        .await,
    );
    assert_eq!(denial.entity_type, "team_member");
    assert_eq!(denial.entity_id.unwrap().expose_secret(), "user-1:team-1");
    assert_eq!(denial.status_code, 422);
    assert_eq!(
        denial.message,
        "Budget has been exceeded! User=user-1 in Team=team-1 Current cost: 30, Max budget: 20"
    );
    let alerts = harness.member_alerts.lock().unwrap().clone();
    let [alert] = alerts.as_slice() else {
        panic!("one member alert, got {alerts:?}");
    };
    assert_eq!(alert.kind, BudgetAlertKind::MaxBudgetAlert);
    assert_eq!(alert.event_group, EventGroup::TeamMember);
    assert_eq!(alert.user_email.as_deref(), Some("member@example.com"));
    assert_eq!(alert.team_id.as_deref(), Some("team-1"));
    assert_eq!(alert.team_alias.as_deref(), Some("Alpha team"));
    assert_eq!(alert.organization_id.as_deref(), Some("org-team"));
    assert_eq!(alert.user_id.as_deref(), Some("user-1"));
}

#[rstest]
#[case::no_membership(Membership::Loaded(None))]
#[case::no_budget_row(loaded(r#"{"user_id":"user-1","team_id":"team-1"}"#))]
#[case::empty_allowlist(loaded(
    r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":[]}}"#
))]
#[case::null_allowlist(loaded(
    r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":null}}"#
))]
#[tokio::test]
async fn member_scope_absent_inherits(#[case] membership: Membership) {
    let harness = Harness::default();
    let row = team(r#"{"team_id":"team-1"}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::One("any-model".to_owned());
    run(
        &harness,
        &request(Some(&row), Some(&token), &model, membership),
    )
    .await
    .unwrap();
}

#[rstest]
#[case::alias_target(Some("my-alias"), true)]
#[case::listed(Some("gpt-4o"), true)]
#[case::unlisted(Some("claude-3"), false)]
#[case::no_model_skips_scope(None, true)]
#[tokio::test]
async fn member_alias_scope(#[case] requested: Option<&str>, #[case] allowed: bool) {
    let harness = Harness::default();
    let row = team(r#"{"team_id":"team-1"}"#);
    let token = key(
        r#"{"token":"sk-member","user_id":"user-1","team_id":"team-1","aliases":{"my-alias":"gpt-4o"}}"#,
    );
    let model = requested.map_or(RequestedModels::None, |name| {
        RequestedModels::One(name.to_owned())
    });
    let membership = loaded(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#,
    );
    let result = run(
        &harness,
        &request(Some(&row), Some(&token), &model, membership),
    )
    .await;
    assert_eq!(result.is_ok(), allowed, "{result:?}");
}

#[tokio::test]
async fn member_denial_safe_message() {
    let harness = Harness::default();
    let row = team(r#"{"team_id":"team-1"}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::One("claude-3".to_owned());
    let membership = loaded(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#,
    );
    let result = run(
        &harness,
        &request(Some(&row), Some(&token), &model, membership),
    )
    .await;
    let Err(TeamBudgetError::ModelDenied(denial)) = result else {
        panic!("expected the member model denial, got {result:?}");
    };
    let exception = denial.as_proxy_exception();
    assert_eq!(exception.message, model_denied_client_message("claude-3"));
    assert_eq!(exception.error_type, "team_model_access_denied");
    assert_eq!(exception.param.as_deref(), Some("model"));
    assert_eq!(exception.code, "403");
    let internal = denial.sanitized_internal_message();
    assert!(internal.contains("User=user-1, Team=team-1"), "{internal}");
    assert!(internal.contains(r#"["gpt-4o"]"#), "{internal}");
    assert!(!exception.message.contains("user-1"));
    // the scope denial comes before any budget read
    assert!(harness.reads().is_empty());
}

#[tokio::test]
async fn nonproxy_model_error_propagates() {
    let harness = Harness::default();
    let row = team(r#"{"team_id":"team-1"}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::One("gpt-4o".to_owned());
    let scoped = r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#;
    let depth = admit_at(
        &harness,
        &request(Some(&row), Some(&token), &model, loaded(scoped)),
        0,
    )
    .await;
    assert!(
        matches!(&depth, Err(TeamBudgetError::RecurseDepth { model: named }) if named == "gpt-4o"),
        "{depth:?}"
    );
    let failing = Harness::default().failing();
    let lookup = run(
        &failing,
        &request(Some(&row), Some(&token), &model, Membership::Unloaded),
    )
    .await;
    assert!(
        matches!(
            lookup,
            Err(TeamBudgetError::Membership(MembershipLoadError::NoDatabase))
        ),
        "{lookup:?}"
    );
}

#[tokio::test]
async fn member_cap_overflow_reads_then_admits() {
    // Decimal::MAX plus an active increase overflows, which LiteLLM's double would hold as a finite cap no spend reaches
    let harness = Harness::spending(&["1000000"]);
    let row = team(r#"{"team_id":"team-1"}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    let membership = loaded(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"max_budget":79228162514264337593543950335,"temp_budget_increase":5,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    run(
        &harness,
        &request(Some(&row), Some(&token), &model, membership),
    )
    .await
    .unwrap();
    let reads = harness.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].counter_key, "spend:team_member:user-1:team-1");
    assert_eq!(reads[0].max_budget, None);
    assert!(harness.events().iter().all(|event| event != "member alert"));
}

#[tokio::test]
async fn member_cap_negative_overflow_denies() {
    // LiteLLM's double sum stays a finite negative cap, so a spend above it is denied instead of admitted
    let harness = Harness::spending(&["0"]);
    let row = team(r#"{"team_id":"team-1"}"#);
    let token = key(MEMBER_KEY);
    let model = RequestedModels::None;
    let membership = loaded(
        r#"{"user_id":"user-1","team_id":"team-1","litellm_budget_table":{"max_budget":-79228162514264337593543950335,"temp_budget_increase":-1,"temp_budget_expiry":"2027-01-01T00:00:00Z"}}"#,
    );
    let denial = denial_of(
        run(
            &harness,
            &request(Some(&row), Some(&token), &model, membership),
        )
        .await,
    );
    assert_eq!(denial.max_budget, Decimal::MIN);
    assert_eq!(denial.current_cost, Decimal::ZERO);
    let reads = harness.reads();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].max_budget, Some(Decimal::MIN));
}
