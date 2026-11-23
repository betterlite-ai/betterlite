use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::vec::IntoIter;

use async_trait::async_trait;
use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::judge_primitives::provider_qualified::ProviderCache;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_secret_managers::secret_helpers::SecretManagerReadState;
use betterlite_secret_managers::secret_types::SecretAccessMode;
use chrono::{DateTime, Duration, TimeZone as _, Utc};
use rust_decimal::Decimal;
use thiserror::Error;
use uuid::Uuid;

use crate::judge_targets::call::JudgeCall;
use crate::judge_targets::error::JudgeDispatchError;
use crate::judge_targets::sdk::JudgeSdk;
use crate::model_visibility::testing::{Row, Snapshot};
use crate::plugin_routes::routes::Identity;
use crate::shadow_eval_job_loading::jobs::ShadowEvalDirection;
use crate::shadow_eval_lifecycle::body::decode_shadow_body;
use crate::shadow_eval_lifecycle::error::LifecycleError;
use crate::shadow_eval_lifecycle::start::{StartCall, StartDeps, start_shadow_eval};
use crate::shadow_eval_lifecycle::store::{
    ClaimedTarget, InsertRejection, NewLeg, ShadowLifecycleStore, TargetKeyRow, TargetRows,
    TargetTeamRow, TargetUserRow,
};
use crate::shadow_eval_models::ShadowEvalTargetType;
use crate::shadow_eval_models::job::ShadowJobResponse;
use crate::shadow_eval_validation::request::{SHADOW_EVAL_TURN_VALVE, StartShadowEvalRequest};
use crate::shadow_eval_validation::router_view::{AnthropicCredentials, ShadowEvalRouter};
use crate::shadow_result_aggregation::legs::LegRow;
use crate::shadow_result_aggregation::responses::LegAttemptCount;
use crate::shadow_result_aggregation::targets::target_type_name;

pub(super) const JUDGE: &str = "judge-model";
pub(super) const TIER: &str = "tier-model";
pub(super) const ROUTER: &str = "shadow-router";
pub(super) const JUDGE_MODEL: &str = "openai/gpt-4o-mini";
pub(super) const TIER_MODEL: &str = "anthropic/claude-sonnet-5";
pub(super) const BASELINE: &str = "baseline-model";

pub(super) const ALL_TARGETS: &str = r#"{"router_names": ["shadow-router"], "api_key_ids": ["key-1"], "team_ids": ["team-1"], "user_ids": ["user-1"], "judge_model": "judge-model", "shadow_percentage": 10, "max_budget": 10}"#;
pub(super) const KEY_ONLY: &str = r#"{"router_names": ["shadow-router"], "api_key_ids": ["key-1"], "judge_model": "judge-model", "shadow_percentage": 10, "max_budget": 10}"#;
pub(super) const REVERSE_KEY: &str = r#"{"router_names": ["shadow-router"], "api_key_ids": ["key-1"], "judge_model": "judge-model", "shadow_percentage": 10, "max_budget": 10, "direction": "reverse", "baseline_model": "baseline-model"}"#;

// the clock every fake and every response reads, so ends_at arithmetic is a hand-checkable value
pub(super) fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 23, 12, 0, 0).unwrap()
}

pub(super) fn request(body: &str) -> StartShadowEvalRequest {
    decode_shadow_body(body.as_bytes()).unwrap()
}

pub(super) fn minted(count: u128) -> IntoIter<Uuid> {
    (1..=count)
        .map(Uuid::from_u128)
        .collect::<Vec<Uuid>>()
        .into_iter()
}

pub(super) fn identity(role: Option<&str>) -> Identity {
    Identity {
        user_id: Some("admin-user".to_owned()),
        user_role: role.map(str::to_owned),
    }
}

pub(super) fn targets() -> TargetRows {
    TargetRows {
        keys: vec![TargetKeyRow {
            token: "key-1".to_owned(),
            key_alias: Some("prod-key".to_owned()),
            key_name: Some("sk-...abcd".to_owned()),
            team_id: Some("team-1".to_owned()),
        }],
        teams: vec![TargetTeamRow {
            team_id: "team-1".to_owned(),
            team_alias: Some("Team One".to_owned()),
        }],
        users: vec![TargetUserRow {
            user_id: "user-1".to_owned(),
            user_email: Some("user@example.com".to_owned()),
        }],
    }
}

// one leg of a job started earlier, its (created_at, ends_at) window open unless a test closes it
pub(super) fn leg(
    group_id: &str,
    target_type: ShadowEvalTargetType,
    target_id: &str,
    direction: ShadowEvalDirection,
    window: (DateTime<Utc>, DateTime<Utc>),
) -> LegRow {
    let (created_at, ends_at) = window;
    LegRow {
        id: format!("{group_id}-{target_id}"),
        group_id: group_id.to_owned(),
        target_type,
        target_id: target_id.to_owned(),
        router_name: ROUTER.to_owned(),
        router_names: vec![ROUTER.to_owned()],
        models: Vec::new(),
        direction,
        baseline_model: None,
        judge_model: JUDGE.to_owned(),
        shadow_percentage: 10.0,
        max_turns: i64::from(SHADOW_EVAL_TURN_VALVE),
        max_budget: Some(Decimal::new(1000, 2)),
        created_at,
        ends_at,
        stopped_at: None,
        stopped_by: None,
    }
}

pub(super) struct FakeAttempt {
    pub(super) job_id: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) spend: Decimal,
}

fn within(attempt: &FakeAttempt, leg: &LegRow) -> bool {
    leg.stopped_at
        .is_none_or(|stamp| attempt.created_at <= stamp)
}

pub(super) fn attempt_count(attempts: &[FakeAttempt], leg: &LegRow) -> i64 {
    let live = attempts
        .iter()
        .filter(|attempt| attempt.job_id == leg.id && within(attempt, leg));
    i64::try_from(live.count()).unwrap()
}

pub(super) fn attempt_spend(attempts: &[FakeAttempt], leg: &LegRow) -> Decimal {
    attempts
        .iter()
        .filter(|attempt| attempt.job_id == leg.id && within(attempt, leg))
        .map(|attempt| attempt.spend)
        .sum()
}

// job-1's key and team legs, two attempts on the key and one on the team, the fixture the stop tests read
pub(super) fn stopping() -> FakeStore {
    let opened = now().checked_sub_signed(Duration::days(1)).unwrap();
    let closed = now().checked_add_signed(Duration::days(6)).unwrap();
    let key = leg(
        "job-1",
        ShadowEvalTargetType::Key,
        "key-1",
        ShadowEvalDirection::Forward,
        (opened, closed),
    );
    let team = leg(
        "job-1",
        ShadowEvalTargetType::Team,
        "team-1",
        ShadowEvalDirection::Forward,
        (opened, closed),
    );
    let attempts = vec![
        FakeAttempt {
            job_id: key.id.clone(),
            created_at: opened,
            spend: Decimal::new(100, 2),
        },
        FakeAttempt {
            job_id: key.id.clone(),
            created_at: now(),
            spend: Decimal::new(50, 2),
        },
        FakeAttempt {
            job_id: team.id.clone(),
            created_at: now(),
            spend: Decimal::new(25, 2),
        },
    ];
    FakeStore::new(now())
        .with_targets(targets())
        .with_legs(vec![key, team])
        .with_attempts(attempts)
}

fn budget_open(leg: &LegRow, attempts: &[FakeAttempt]) -> bool {
    leg.max_budget
        .is_none_or(|budget| attempt_spend(attempts, leg) < budget)
}

#[derive(Debug, Error)]
#[error("{message}")]
pub(super) struct FakeError {
    pub(super) message: String,
}

impl FakeError {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }
}

// an in-memory LiteLLM_ShadowEvalJob table running the same sweep, claim, insert and stop statements
pub(super) struct FakeStore {
    pub(super) targets: TargetRows,
    pub(super) now: DateTime<Utc>,
    pub(super) legs: Mutex<Vec<LegRow>>,
    pub(super) attempts: Mutex<Vec<FakeAttempt>>,
    pub(super) seeds_fail: bool,
    pub(super) create_conflict: bool,
    pub(super) calls: Mutex<Vec<String>>,
}

impl FakeStore {
    pub(super) fn new(now: DateTime<Utc>) -> Self {
        Self {
            targets: TargetRows::default(),
            now,
            legs: Mutex::new(Vec::new()),
            attempts: Mutex::new(Vec::new()),
            seeds_fail: false,
            create_conflict: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn with_targets(mut self, targets: TargetRows) -> Self {
        self.targets = targets;
        self
    }

    pub(super) fn with_legs(self, legs: Vec<LegRow>) -> Self {
        *self.legs.lock().unwrap() = legs;
        self
    }

    pub(super) fn with_attempts(self, attempts: Vec<FakeAttempt>) -> Self {
        *self.attempts.lock().unwrap() = attempts;
        self
    }

    pub(super) fn stored_legs(&self) -> Vec<LegRow> {
        self.legs.lock().unwrap().clone()
    }

    pub(super) fn recorded(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl ShadowLifecycleStore for FakeStore {
    type Error = FakeError;

    async fn load_targets(
        &self,
        keys: &[String],
        teams: &[String],
        users: &[String],
    ) -> Result<TargetRows, Self::Error> {
        self.calls.lock().unwrap().push("load".to_owned());
        Ok(TargetRows {
            keys: self
                .targets
                .keys
                .iter()
                .filter(|row| keys.contains(&row.token))
                .cloned()
                .collect(),
            teams: self
                .targets
                .teams
                .iter()
                .filter(|row| teams.contains(&row.team_id))
                .cloned()
                .collect(),
            users: self
                .targets
                .users
                .iter()
                .filter(|row| users.contains(&row.user_id))
                .cloned()
                .collect(),
        })
    }

    async fn sweep_finished(
        &self,
        ids: &[String],
        target_type: ShadowEvalTargetType,
    ) -> Result<(), Self::Error> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("sweep:{}", target_type_name(target_type)));
        let attempts = self.attempts.lock().unwrap();
        let mut legs = self.legs.lock().unwrap();
        for leg in &mut *legs {
            if leg.target_type != target_type
                || !ids.contains(&leg.target_id)
                || leg.stopped_at.is_some()
            {
                continue;
            }
            let finished = leg.ends_at <= self.now
                || attempt_count(&attempts, leg) >= leg.max_turns
                || !budget_open(leg, &attempts);
            if finished {
                leg.stopped_at = Some(self.now);
            }
        }
        drop(legs);
        drop(attempts);
        Ok(())
    }

    async fn claimed_targets(
        &self,
        targets: &[(ShadowEvalTargetType, Vec<String>)],
        direction: ShadowEvalDirection,
    ) -> Result<Vec<ClaimedTarget>, Self::Error> {
        self.calls.lock().unwrap().push("claim".to_owned());
        let legs = self.legs.lock().unwrap();
        Ok(legs
            .iter()
            .filter(|leg| {
                leg.stopped_at.is_none()
                    && leg.direction == direction
                    && targets.iter().any(|(target_type, ids)| {
                        *target_type == leg.target_type && ids.contains(&leg.target_id)
                    })
            })
            .map(|leg| ClaimedTarget {
                group_id: leg.group_id.clone(),
                target_type: leg.target_type,
                target_id: leg.target_id.clone(),
            })
            .collect())
    }

    async fn create_legs(&self, legs: &[NewLeg]) -> Result<(), InsertRejection<Self::Error>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("create:{}", legs.len()));
        if self.create_conflict {
            return Err(InsertRejection::UniqueViolation);
        }
        let rows: Vec<LegRow> = legs.iter().map(NewLeg::row).collect();
        self.legs.lock().unwrap().extend(rows);
        Ok(())
    }

    async fn seed_funnel(&self, job_ids: &[String]) -> Result<(), Self::Error> {
        self.record(format!("seed:{}", job_ids.len()));
        if self.seeds_fail {
            return Err(FakeError::new("the funnel table is unavailable"));
        }
        Ok(())
    }

    async fn legs(&self, group_id: &str) -> Result<Vec<LegRow>, Self::Error> {
        self.calls.lock().unwrap().push("legs".to_owned());
        let legs = self.legs.lock().unwrap();
        Ok(legs
            .iter()
            .filter(|leg| leg.group_id == group_id)
            .cloned()
            .collect())
    }

    async fn attempt_stats(&self, job_ids: &[String]) -> Result<Vec<LegAttemptCount>, Self::Error> {
        self.calls.lock().unwrap().push("stats".to_owned());
        let legs = self.legs.lock().unwrap();
        let attempts = self.attempts.lock().unwrap();
        Ok(legs
            .iter()
            .filter(|leg| job_ids.contains(&leg.id))
            .filter_map(|leg| {
                let count = attempt_count(&attempts, leg);
                if count == 0 {
                    return None;
                }
                Some(LegAttemptCount {
                    job_id: leg.id.clone(),
                    attempt_count: count,
                    spend: attempt_spend(&attempts, leg),
                })
            })
            .collect())
    }

    async fn stop_job(
        &self,
        job_id: &str,
        operator: &str,
        stamp: DateTime<Utc>,
    ) -> Result<u64, Self::Error> {
        self.calls.lock().unwrap().push(format!("stop:{job_id}"));
        let attempts = self.attempts.lock().unwrap();
        let mut legs = self.legs.lock().unwrap();
        let group: Vec<usize> = legs
            .iter()
            .enumerate()
            .filter(|(_, leg)| leg.group_id == job_id)
            .map(|(index, _)| index)
            .collect();
        let Some(first) = group.first().copied() else {
            return Ok(0);
        };
        let window_open = legs[first].ends_at > self.now;
        let still_sampling = group.iter().any(|index| {
            let leg = &legs[*index];
            leg.stopped_at.is_none()
                && attempt_count(&attempts, leg) < leg.max_turns
                && budget_open(leg, &attempts)
        });
        if !window_open || !still_sampling {
            return Ok(0);
        }
        for index in group {
            let leg = &mut legs[index];
            if leg.stopped_by.is_none() && leg.ends_at > self.now {
                leg.stopped_by = Some(operator.to_owned());
                if leg.stopped_at.is_none() {
                    leg.stopped_at = Some(stamp);
                }
            }
        }
        drop(legs);
        drop(attempts);
        Ok(1)
    }
}

struct NoSdk;

#[async_trait]
impl JudgeSdk for NoSdk {
    async fn acompletion(&self, _call: JudgeCall) -> Result<Json, JudgeDispatchError> {
        Err(JudgeDispatchError {
            message: "the lifecycle tests never dispatch a judge call".to_owned(),
        })
    }
}

fn unconsulted() -> SecretManagerReadState<'static> {
    SecretManagerReadState {
        client_exists: false,
        settings_exist: false,
        access_mode: SecretAccessMode::ReadOnly,
        hosted_keys: None,
    }
}

fn params(model: &str) -> JsonObject {
    JsonObject::from([("model".to_owned(), Json::Text(model.to_owned()))])
}

// the same router fixture B345 validates against: a judge, a tier, a baseline and one complexity router
fn snapshot() -> Snapshot {
    let info = Json::Object(JsonObject::new());
    let router = JsonObject::from([
        (
            "model".to_owned(),
            Json::Text("auto_router/complexity_router".to_owned()),
        ),
        (
            "complexity_router_default_model".to_owned(),
            Json::Text(TIER.to_owned()),
        ),
    ]);
    let deployments = [
        (JUDGE, params(JUDGE_MODEL)),
        (TIER, params(TIER_MODEL)),
        (BASELINE, params("openai/gpt-4o")),
        (ROUTER, router),
    ];
    let rows: Vec<Row<'_>> = deployments
        .iter()
        .map(|(name, litellm_params)| Row {
            model_name: name,
            litellm_params,
            model_info: &info,
        })
        .collect();
    Snapshot::build(&rows, &JsonObject::new()).unwrap()
}

// the router parts the start fixture validates against, its unused provider box dropped so the start future stays Send
fn router_parts() -> (DeploymentCatalog, AliasMap, RoutingGroupIndex) {
    let Snapshot {
        catalog,
        aliases,
        groups,
        provider,
    } = snapshot();
    drop(provider);
    (catalog, aliases, groups)
}

// the production entry the start route calls, with the live router fixture it validates against
pub(super) async fn start(
    store: &FakeStore,
    request: &StartShadowEvalRequest,
    caller: Option<&str>,
    now: DateTime<Utc>,
    ids: &mut IntoIter<Uuid>,
) -> Result<ShadowJobResponse, LifecycleError> {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let no_names = BTreeSet::new();
    let no_environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: catalog.as_ref(),
        known_model_names: &no_names,
        custom_providers: &no_names,
        environment: &no_environment,
        registry: JSONProviderRegistry::load(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let cache = ProviderCache::default();
    let settings = AnthropicSettings::default();
    let (router_catalog, router_aliases, router_groups) = router_parts();
    let provider = |_model: &str| -> Option<String> { None };
    let credentials = AnthropicCredentials {
        anthropic_key: None,
        api_key: None,
        settings: &settings,
        secret_state: unconsulted(),
    };
    let deps = StartDeps {
        router: Some(ShadowEvalRouter {
            catalog: &router_catalog,
            model_group_alias: &router_aliases,
            routing_groups: &router_groups,
            inferred_provider: &provider,
        }),
        sdk: &NoSdk,
        cache: &cache,
        resolution: &resolution,
        credentials,
    };
    let mut call = StartCall { caller, now, ids };
    start_shadow_eval(store, deps, request, &mut call).await
}

pub(super) fn error_of(result: Result<ShadowJobResponse, LifecycleError>) -> LifecycleError {
    match result {
        Ok(response) => panic!("expected an error, got job {}", response.job.job_id),
        Err(error) => error,
    }
}
