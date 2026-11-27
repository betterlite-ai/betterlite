use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_auth::auth_invalidation::AuthInvalidation;
use betterlite_auth::auth_invalidation::clock::SystemClock;
use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_auth::auth_invalidation::ports::{
    AuthInvalidationCache, DeferredDelete, RedisMarker, SpendCounterCache,
};
use betterlite_auth::auth_invalidation::transport::InvalidationTransport;
use betterlite_auth::auth_key_lookup::jwt::{JwtError, JwtMappingSource};
use betterlite_auth::auth_pubsub::eviction::LocalEviction;
use betterlite_auth::auth_pubsub::message::Invalidation;
use betterlite_auth::auth_read::jwt_rows::{JwtMappingFilter, JwtMappingRow};
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_cache::backend::BackendError;
use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::member_removal_identity::identity::MemberIdentity;
use betterlite_db::team_rows::member::Member;
use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::prometheus::budget_gauges::gauge::GaugeField;
use betterlite_logging::prometheus_label_projection::label_factory_context::PrometheusLabelFactoryContext;
use betterlite_logging::prometheus_label_projection::prometheus_label_factory::LabelSource;
use betterlite_logging::prometheus_label_schema::PrometheusLabelSettings;
use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::{Collector, MetricUpdate, MetricValue};
use betterlite_prometheus::label_value_record::{LabelDumpValue, UserApiKeyLabelValues};
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use chrono::{DateTime, FixedOffset};
use secrecy::{ExposeSecret as _, SecretString};
use tempfile::NamedTempFile;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::audit_events::callbacks::AuditCallbackConfig;
use crate::audit_events::identity::CallerIdentity;
use crate::audit_events::record::AuditLogRecord;
use crate::audit_events::settings::AuditSettings;
use crate::audit_events::sink::AuditClock;
use crate::bulk_team_removal::audit::DeletedKeyAudit;
use crate::bulk_team_removal::error::BulkRemovalError;
use crate::bulk_team_removal::removal::{
    RemovalTables, TeamRemoval, TeamRemovalSession, remove_members_from_team,
};
use crate::bulk_team_removal::service::BulkTeamRemoval;
use crate::key_events::audit::{KeyAuditScheduler, KeyEventAudit};
use crate::key_events::types::KeyEventCaller;
use crate::team_access::access::{OrgRoles, TeamAccess};
use crate::team_access::error::TeamAccessError;
use crate::team_request_contracts::bulk_members::TeamMemberDeleteResult;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// every collaborator appends its step here, so a test reads the whole order of one removal
#[derive(Clone, Default)]
pub(crate) struct EventLog {
    entries: Arc<Mutex<Vec<String>>>,
}

impl EventLog {
    pub(crate) fn push(&self, entry: String) {
        lock(&self.entries).push(entry);
    }

    pub(crate) fn events(&self) -> Vec<String> {
        lock(&self.entries).clone()
    }
}

pub(crate) fn lines(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

pub(crate) fn assert_step_order(log: &[String], steps: &[&str]) {
    let positions: Vec<usize> = steps
        .iter()
        .map(|step| {
            log.iter()
                .position(|event| event.starts_with(step))
                .unwrap_or_else(|| panic!("no {step} event in {log:?}"))
        })
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "{log:?}"
    );
}

#[derive(Clone, Default)]
pub(crate) struct MemoryState {
    pub(crate) teams: BTreeMap<String, LiteLLMTeamTable>,
    pub(crate) users: BTreeMap<String, LiteLLMUserTable>,
    pub(crate) memberships: BTreeSet<(String, String)>,
    pub(crate) keys: BTreeMap<String, LiteLLMVerificationToken>,
    pub(crate) archived: Vec<LiteLLMVerificationToken>,
}

// the in-memory tables speak the same removal trait as Postgres, and the trait's steps call the real selection
#[derive(Clone, Default)]
pub(crate) struct MemorySession {
    pub(crate) log: EventLog,
    pub(crate) state: Arc<Mutex<MemoryState>>,
}

impl MemorySession {
    pub(crate) fn add_team(&self, team: LiteLLMTeamTable) {
        lock(&self.state)
            .teams
            .insert(team.fields.base.team_id.clone(), team);
    }

    pub(crate) fn add_user(&self, user: LiteLLMUserTable) {
        lock(&self.state).users.insert(user.user_id.clone(), user);
    }

    pub(crate) fn add_membership(&self, team_id: &str, user_id: &str) {
        lock(&self.state)
            .memberships
            .insert((team_id.to_owned(), user_id.to_owned()));
    }

    pub(crate) fn add_key(&self, key: LiteLLMVerificationToken) {
        let token = key
            .token
            .as_ref()
            .map(|token| token.expose_secret().clone())
            .unwrap_or_default();
        lock(&self.state).keys.insert(token, key);
    }

    pub(crate) fn roster(&self, team_id: &str) -> Vec<Member> {
        lock(&self.state)
            .teams
            .get(team_id)
            .map(|team| team.fields.base.members_with_roles.clone())
            .unwrap_or_default()
    }

    pub(crate) fn user_teams(&self, user_id: &str) -> Vec<String> {
        lock(&self.state)
            .users
            .get(user_id)
            .map(|user| user.teams.clone())
            .unwrap_or_default()
    }

    pub(crate) fn memberships(&self) -> Vec<(String, String)> {
        lock(&self.state).memberships.iter().cloned().collect()
    }

    pub(crate) fn key_tokens(&self) -> Vec<String> {
        lock(&self.state).keys.keys().cloned().collect()
    }

    pub(crate) fn archived_tokens(&self) -> Vec<String> {
        token_list(&lock(&self.state).archived)
    }
}

fn token_list(keys: &[LiteLLMVerificationToken]) -> Vec<String> {
    keys.iter()
        .filter_map(|key| {
            key.token
                .as_ref()
                .map(|token| token.expose_secret().clone())
        })
        .collect()
}

fn request_label(member: &MemberIdentity) -> String {
    match (&member.user_id, &member.user_email) {
        (Some(user_id), _) => user_id.clone(),
        (None, Some(user_email)) => user_email.clone(),
        (None, None) => String::new(),
    }
}

#[async_trait]
impl RemovalTables for MemorySession {
    async fn lock_team(&self, team_id: &str) -> Result<(), BulkRemovalError> {
        self.log.push(format!("lock:{team_id}"));
        Ok(())
    }

    async fn roster(&self, team_id: &str) -> Result<Option<Vec<Member>>, BulkRemovalError> {
        self.log.push(format!("roster:{team_id}"));
        Ok(lock(&self.state)
            .teams
            .get(team_id)
            .map(|team| team.fields.base.members_with_roles.clone()))
    }

    async fn users(
        &self,
        user_ids: &[String],
        user_emails: &[String],
    ) -> Result<Vec<LiteLLMUserTable>, BulkRemovalError> {
        self.log.push(format!(
            "users:{};{}",
            user_ids.join(","),
            user_emails.join(",")
        ));
        let users = lock(&self.state).users.clone();
        Ok(users
            .into_values()
            .filter(|user| {
                user_ids.contains(&user.user_id)
                    || user
                        .user_email
                        .as_ref()
                        .is_some_and(|email| user_emails.contains(email))
            })
            .collect())
    }

    async fn tokens(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<Vec<LiteLLMVerificationToken>, BulkRemovalError> {
        self.log.push(format!("tokens:{}", user_ids.join(",")));
        let keys = lock(&self.state).keys.clone();
        Ok(keys
            .into_values()
            .filter(|key| {
                key.team_id.as_deref() == Some(team_id)
                    && key
                        .user_id
                        .as_deref()
                        .is_some_and(|user| user_ids.iter().any(|id| id == user))
            })
            .collect())
    }

    async fn write_roster(&self, team_id: &str, kept: &[Member]) -> Result<(), BulkRemovalError> {
        self.log
            .push(format!("write-roster:{team_id}:{}", kept.len()));
        let mut state = lock(&self.state);
        let Some(team) = state.teams.get_mut(team_id) else {
            return Err(BulkRemovalError::MissingRow);
        };
        team.fields.base.members_with_roles = kept.to_vec();
        Ok(())
    }

    async fn write_user_teams(
        &self,
        user_id: &str,
        teams: &[String],
    ) -> Result<(), BulkRemovalError> {
        self.log
            .push(format!("write-teams:{user_id}:{}", teams.join(",")));
        let mut state = lock(&self.state);
        let Some(user) = state.users.get_mut(user_id) else {
            return Err(BulkRemovalError::MissingRow);
        };
        user.teams = teams.to_vec();
        Ok(())
    }

    async fn delete_memberships(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<(), BulkRemovalError> {
        self.log
            .push(format!("delete-memberships:{}", user_ids.join(",")));
        lock(&self.state)
            .memberships
            .retain(|(team, user)| team != team_id || !user_ids.iter().any(|id| id == user));
        Ok(())
    }

    async fn archive_tokens(
        &self,
        keys: &[LiteLLMVerificationToken],
        _caller: &CallerIdentity,
    ) -> Result<(), BulkRemovalError> {
        self.log
            .push(format!("archive:{}", token_list(keys).join(",")));
        lock(&self.state).archived.extend(keys.iter().cloned());
        Ok(())
    }

    async fn delete_tokens(
        &self,
        team_id: &str,
        user_ids: &[String],
    ) -> Result<(), BulkRemovalError> {
        self.log.push(format!("delete-keys:{}", user_ids.join(",")));
        lock(&self.state).keys.retain(|_, key| {
            key.team_id.as_deref() != Some(team_id)
                || !key
                    .user_id
                    .as_deref()
                    .is_some_and(|user| user_ids.iter().any(|id| id == user))
        });
        Ok(())
    }
}

#[async_trait]
impl TeamRemovalSession for MemorySession {
    async fn find_team(&self, team_id: &str) -> Result<Option<LiteLLMTeamTable>, BulkRemovalError> {
        self.log.push(format!("lookup:{team_id}"));
        Ok(lock(&self.state).teams.get(team_id).cloned())
    }

    async fn remove_members(
        &self,
        team_id: &str,
        members: &[MemberIdentity],
        caller: &CallerIdentity,
        jwt: &dyn JwtMappingSource,
    ) -> Result<TeamRemoval, BulkRemovalError> {
        let labels: Vec<String> = members.iter().map(request_label).collect();
        self.log.push(format!("transaction:{}", labels.join(",")));
        remove_members_from_team(self, team_id, members, caller, jwt).await
    }
}

#[derive(Default)]
pub(crate) struct OrgAdmins {
    admins: Mutex<BTreeSet<(String, String)>>,
}

impl OrgAdmins {
    pub(crate) fn grant(&self, user_id: &str, organization_id: &str) {
        lock(&self.admins).insert((user_id.to_owned(), organization_id.to_owned()));
    }
}

#[async_trait]
impl OrgRoles for OrgAdmins {
    async fn is_org_admin(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<bool, TeamAccessError> {
        Ok(lock(&self.admins).contains(&(user_id.to_owned(), organization_id.to_owned())))
    }
}

pub(crate) struct StaticJwt {
    pub(crate) log: EventLog,
    pub(crate) rows: Arc<Mutex<Vec<JwtMappingRow>>>,
}

impl StaticJwt {
    pub(crate) fn add_mapping(
        &self,
        token: &str,
        claim_name: &str,
        claim_value: &str,
        issuer: &str,
    ) {
        lock(&self.rows).push(JwtMappingRow {
            token: token.to_owned(),
            jwt_issuer: issuer.to_owned(),
            jwt_claim_name: claim_name.to_owned(),
            jwt_claim_value: claim_value.to_owned(),
        });
    }
}

#[async_trait]
impl JwtMappingSource for StaticJwt {
    async fn find_active_mapping(
        &self,
        _filter: &JwtMappingFilter,
    ) -> Result<Option<JwtMappingRow>, JwtError> {
        Ok(None)
    }

    async fn mappings_for_tokens(
        &self,
        hashed_tokens: &[String],
    ) -> Result<Vec<JwtMappingRow>, JwtError> {
        self.log
            .push(format!("mappings:{}", hashed_tokens.join(",")));
        let rows = lock(&self.rows).clone();
        Ok(rows
            .into_iter()
            .filter(|row| hashed_tokens.contains(&row.token))
            .collect())
    }
}

pub(crate) struct LogAudit {
    pub(crate) log: EventLog,
    pub(crate) failing: AtomicBool,
}

impl DeletedKeyAudit for LogAudit {
    fn schedule_deleted(
        &self,
        keys: &[LiteLLMVerificationToken],
        caller: &KeyEventCaller,
    ) -> Result<(), BulkRemovalError> {
        if self.failing.load(Relaxed) {
            self.log.push(format!("audit-failed:{}", keys.len()));
            return Err(BulkRemovalError::MissingRow);
        }
        let user = caller.identity.user_id.as_deref().unwrap_or_default();
        for token in token_list(keys) {
            self.log.push(format!("audit:{token}:{user}"));
        }
        Ok(())
    }
}

pub(crate) struct RecordingAuth {
    pub(crate) log: EventLog,
    pub(crate) fail_delete_sync: AtomicBool,
    pub(crate) fail_delete_memory: AtomicBool,
}

fn memory_failure() -> InvalidationError {
    InvalidationError::Memory(Box::new(BackendError::Unsupported("test")))
}

#[async_trait]
impl AuthInvalidationCache for RecordingAuth {
    fn redis_handle(&self, _key: &str) -> Option<Arc<dyn RedisMarker>> {
        None
    }

    fn management_ttl(&self) -> f64 {
        60.0
    }

    async fn write_management(
        &self,
        _key: &str,
        _payload: &Json,
        _ttl: Option<f64>,
    ) -> Result<(), InvalidationError> {
        Ok(())
    }

    fn delete_sync(&self, key: &str) -> Result<(), InvalidationError> {
        self.log.push(format!("key-cache:{key}"));
        if self.fail_delete_sync.load(Relaxed) {
            return Err(memory_failure());
        }
        Ok(())
    }

    async fn delete_async(&self, _key: &str) -> Result<(), InvalidationError> {
        Ok(())
    }

    async fn delete_pre_call(&self, _key: &str) -> Result<DeferredDelete, InvalidationError> {
        Ok(DeferredDelete::Unqueued)
    }

    fn delete_memory(&self, key: &str) -> Result<(), BackendError> {
        self.log.push(format!("memory:{key}"));
        if self.fail_delete_memory.load(Relaxed) {
            return Err(BackendError::Unsupported("test"));
        }
        Ok(())
    }

    fn delete_redis(&self, _key: &str) -> Result<(), RedisAdminError> {
        Ok(())
    }
}

pub(crate) struct NoopSpend;

#[async_trait]
impl SpendCounterCache for NoopSpend {
    fn redis_handle(&self) -> Option<Arc<dyn RedisMarker>> {
        None
    }

    fn set_memory(&self, _key: &str, _value: f64, _ttl: f64) -> Result<(), BackendError> {
        Ok(())
    }

    async fn set_redis(
        &self,
        _key: &str,
        _value: f64,
        _ttl: f64,
    ) -> Result<(), RedisCacheValuesError> {
        Ok(())
    }

    async fn delete_redis(&self, _key: &str) -> Result<(), RedisAdminError> {
        Ok(())
    }
}

pub(crate) struct RecordingTransport {
    pub(crate) log: EventLog,
}

#[async_trait]
impl InvalidationTransport for RecordingTransport {
    async fn publish(&self, cache_key: &str, _new_value: Option<f64>, _ttl: Option<f64>) {
        self.log.push(format!("publish:{cache_key}"));
    }

    async fn evict_and_broadcast(&self, cache_keys: &[String], local: &dyn LocalEviction) {
        for key in cache_keys {
            let invalidation = Invalidation {
                cache_key: key.clone(),
                new_value: None,
                ttl: None,
            };
            let applied = local.apply(invalidation);
            let step = if applied.is_ok() {
                format!("mapping:{key}")
            } else {
                format!("mapping-failed:{key}")
            };
            self.log.push(step);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct LogGauge {
    pub(crate) log: EventLog,
    pub(crate) labels: Vec<(String, String)>,
}

impl Collector for LogGauge {
    fn labels(&self, labels: &[(&str, &str)]) -> Self {
        Self {
            log: self.log.clone(),
            labels: labels
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    fn update(&self, update: MetricUpdate) {
        let MetricUpdate::Set(MetricValue::Finite(value)) = update else {
            return;
        };
        let rendered = self
            .labels
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<String>>()
            .join(",");
        self.log.push(format!("metric:{rendered}:{value}"));
    }
}

pub(crate) struct Fixture {
    pub(crate) log: EventLog,
    pub(crate) session: MemorySession,
    pub(crate) org: Arc<OrgAdmins>,
    pub(crate) access: TeamAccess,
    pub(crate) jwt: StaticJwt,
    pub(crate) audit: LogAudit,
    pub(crate) auth: Arc<RecordingAuth>,
    pub(crate) spend: Arc<NoopSpend>,
    pub(crate) transport: Arc<RecordingTransport>,
    pub(crate) invalidation: AuthInvalidation,
    pub(crate) gauge: GaugeField<LogGauge>,
    pub(crate) settings: PrometheusLabelSettings,
}

pub(crate) fn fixture() -> Fixture {
    let log = EventLog::default();
    let org = Arc::new(OrgAdmins::default());
    let org_roles: Arc<dyn OrgRoles> = org.clone();
    let jwt = StaticJwt {
        log: log.clone(),
        rows: Arc::default(),
    };
    let audit = LogAudit {
        log: log.clone(),
        failing: AtomicBool::new(false),
    };
    let auth = Arc::new(RecordingAuth {
        log: log.clone(),
        fail_delete_sync: AtomicBool::new(false),
        fail_delete_memory: AtomicBool::new(false),
    });
    let spend = Arc::new(NoopSpend);
    let transport = Arc::new(RecordingTransport { log: log.clone() });
    let invalidation = AuthInvalidation {
        auth: auth.clone(),
        usage: None,
        spend: spend.clone(),
        transport: transport.clone(),
        clock: Arc::new(SystemClock),
        access_group_ttl: 0.0,
    };
    let gauge = GaugeField {
        metric_name: "litellm_team_members_metric",
        supported_labels: vec!["team".to_owned(), "team_alias".to_owned()],
        collector: LogGauge {
            log: log.clone(),
            labels: Vec::new(),
        },
    };
    Fixture {
        log,
        session: MemorySession::default(),
        org,
        access: TeamAccess { org_roles },
        jwt,
        audit,
        auth,
        spend,
        transport,
        invalidation,
        gauge,
        settings: PrometheusLabelSettings::with_defaults(),
    }
}

impl Fixture {
    pub(crate) fn labels(&self) -> LabelSource<'_> {
        LabelSource::Values {
            settings: &self.settings,
            end_user_settings: EndUserCostTrackingSettings::default(),
        }
    }

    pub(crate) fn team_access(&self) -> TeamAccess {
        TeamAccess {
            org_roles: self.access.org_roles.clone(),
        }
    }

    pub(crate) fn service<'deps>(
        &'deps self,
        session: &'deps dyn TeamRemovalSession,
    ) -> BulkTeamRemoval<'deps, LogGauge> {
        BulkTeamRemoval {
            session,
            access: &self.access,
            jwt: &self.jwt,
            audit: &self.audit,
            invalidation: &self.invalidation,
            gauge: Some(&self.gauge),
            labels: self.labels(),
        }
    }

    pub(crate) async fn remove_on(
        &self,
        session: &dyn TeamRemovalSession,
        team_id: &str,
        members: &[MemberIdentity],
        auth: &UserAPIKeyAuth,
    ) -> Result<Vec<TeamMemberDeleteResult>, BulkRemovalError> {
        self.service(session)
            .bulk_remove_team_members(team_id, members, auth)
            .await
    }

    pub(crate) async fn remove(
        &self,
        team_id: &str,
        members: &[MemberIdentity],
        auth: &UserAPIKeyAuth,
    ) -> Result<Vec<TeamMemberDeleteResult>, BulkRemovalError> {
        self.remove_on(&self.session, team_id, members, auth).await
    }
}

pub(crate) fn fixture_with_team(team: LiteLLMTeamTable) -> Fixture {
    let built = fixture();
    built.session.add_team(team);
    built
}

pub(crate) fn member(user_id: &str) -> Member {
    serde_json::from_value(serde_json::json!({"user_id": user_id, "role": "user"})).unwrap()
}

pub(crate) fn email_member(user_email: &str) -> Member {
    serde_json::from_value(serde_json::json!({"user_email": user_email, "role": "user"})).unwrap()
}

pub(crate) fn member_with_email(user_id: &str, user_email: &str) -> Member {
    serde_json::from_value(serde_json::json!({
        "user_id": user_id,
        "user_email": user_email,
        "role": "user",
    }))
    .unwrap()
}

pub(crate) fn team(team_id: &str, members: &[Member]) -> LiteLLMTeamTable {
    serde_json::from_value(serde_json::json!({"team_id": team_id, "members_with_roles": members}))
        .unwrap()
}

pub(crate) fn team_with_org(
    team_id: &str,
    organization_id: &str,
    members: &[Member],
) -> LiteLLMTeamTable {
    serde_json::from_value(serde_json::json!({
        "team_id": team_id,
        "organization_id": organization_id,
        "members_with_roles": members,
    }))
    .unwrap()
}

pub(crate) fn user(user_id: &str, user_email: Option<&str>, teams: &[&str]) -> LiteLLMUserTable {
    serde_json::from_value(serde_json::json!({
        "user_id": user_id,
        "user_email": user_email,
        "teams": teams,
    }))
    .unwrap()
}

pub(crate) fn key(token: &str, team_id: &str, user_id: &str) -> LiteLLMVerificationToken {
    serde_json::from_value(serde_json::json!({
        "token": token,
        "team_id": team_id,
        "user_id": user_id,
        "key_name": token,
    }))
    .unwrap()
}

pub(crate) fn identity_id(user_id: &str) -> MemberIdentity {
    MemberIdentity {
        user_id: Some(user_id.to_owned()),
        user_email: None,
    }
}

pub(crate) fn identity(user_id: Option<&str>, user_email: Option<&str>) -> MemberIdentity {
    MemberIdentity {
        user_id: user_id.map(str::to_owned),
        user_email: user_email.map(str::to_owned),
    }
}

pub(crate) fn proxy_admin() -> UserAPIKeyAuth {
    auth(&[
        ("user_id", "admin"),
        ("user_role", "proxy_admin"),
        ("token", "sk-admin"),
    ])
}

pub(crate) fn member_auth(user_id: &str) -> UserAPIKeyAuth {
    auth(&[("user_id", user_id), ("token", "sk-user")])
}

fn auth(fields: &[(&str, &str)]) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), JsonData::String((*value).to_owned())))
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

pub(crate) fn mismatched_context(
    settings: &PrometheusLabelSettings,
) -> PrometheusLabelFactoryContext {
    PrometheusLabelFactoryContext::new(
        &UserApiKeyLabelValues::from_entries(&[(
            "team".to_owned(),
            LabelDumpValue::String("other-team".to_owned()),
        )]),
        settings,
        EndUserCostTrackingSettings::default(),
    )
    .unwrap()
}

pub(crate) fn capture_log() -> (DefaultGuard, NamedTempFile) {
    let log = NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    (set_default(subscriber), log)
}

pub(crate) fn logged_text(log: &NamedTempFile) -> String {
    fs::read_to_string(log.path()).unwrap()
}

#[derive(Default)]
pub(crate) struct RecordingScheduler {
    pub(crate) records: Mutex<Vec<AuditLogRecord>>,
}

impl RecordingScheduler {
    pub(crate) fn records(&self) -> Vec<AuditLogRecord> {
        lock(&self.records).clone()
    }
}

impl KeyAuditScheduler for RecordingScheduler {
    fn schedule(&self, record: AuditLogRecord) {
        lock(&self.records).push(record);
    }
}

pub(crate) struct TestSettings {
    pub(crate) enabled: bool,
}

impl AuditSettings for TestSettings {
    fn is_audit_logging_enabled(&self, explicit: Option<bool>) -> bool {
        explicit.unwrap_or(self.enabled)
    }

    fn audit_log_callbacks(&self) -> &[AuditCallbackConfig] {
        &[]
    }

    fn s3_audit_callback_params(&self) -> Option<&JsonValue> {
        None
    }
}

pub(crate) struct FixedClock;

impl AuditClock for FixedClock {
    fn now(&self) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-01-02T03:04:05+00:00").unwrap()
    }
}

pub(crate) fn key_event_audit(scheduler: Arc<RecordingScheduler>, enabled: bool) -> KeyEventAudit {
    let scheduled: Arc<dyn KeyAuditScheduler> = scheduler;
    KeyEventAudit {
        settings: Arc::new(TestSettings { enabled }),
        clock: Arc::new(FixedClock),
        new_id: Arc::new(|| "audit-1".to_owned()),
        scheduler: scheduled,
        litellm_proxy_admin_name: None,
    }
}

pub(crate) fn key_event_caller() -> KeyEventCaller {
    KeyEventCaller {
        identity: CallerIdentity {
            user_id: Some("admin".to_owned()),
            api_key: Some(SecretString::from("sk-admin")),
            metadata: None,
            team_metadata: None,
        },
        token: Some(SecretString::from("sk-admin")),
    }
}
