use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::membership::LiteLLMTeamMembership;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use tokio::runtime::{Builder, Runtime};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::agent_access_groups::ceiling::AgentAccessGroupCeiling;
use crate::agent_access_groups::error::AccessGroupsError;
use crate::agent_caller::error::AgentCallerError;
use crate::agent_policy::AgentResponse;
use crate::agent_policy::caller::AgentCaller;
use crate::auth_end_users::defaults::MemberBudgetError;
use crate::auth_key_budgets::spend::{SpendReadError, SpendReader};
use crate::auth_model_grants::access_source::AgentAccessSource;
use crate::auth_model_grants::error::AgentModelGrantsError;
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::pattern::UnresolvedProviderInference;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::budget_primitives::window_start::DurationParser;
use crate::current_spend::SpendQuery;
use crate::entity_model_access::resolved::ResolvedDeps;
use crate::membership_load::error::MembershipLoadError;
use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use crate::team_budget_admission::TeamAdmission;
use crate::team_budget_admission::sources::{DefaultBudgetSource, MembershipSource};

pub(super) const AGENT_KEY: &str = r#"{"token":"sk-agent","agent_id":"agent-1"}"#;
pub(super) const AGENT_TEAM_KEY: &str =
    r#"{"token":"sk-agent","agent_id":"agent-1","team_id":"team-1"}"#;
pub(super) const AGENT_ALIAS_KEY: &str = r#"{"token":"sk-agent","agent_id":"agent-1","team_id":"team-1","team_model_aliases":{"alias":"gpt-4o"}}"#;
pub(super) const AGENT_MODEL_ALIAS_KEY: &str =
    r#"{"token":"sk-agent","agent_id":"agent-1","aliases":{"fast":"gpt-4o"}}"#;
pub(super) const CALLER_KEY: &str =
    r#"{"token":"sk-caller","user_id":"caller-1","team_id":"team-1"}"#;
pub(super) const POLICY: &str =
    r#"{"agent_id":"agent-1","agent_name":"Agent","agent_card_params":{}}"#;
pub(super) const ALLOWING_TEAM: &str =
    r#"{"team_id":"team-1","models":["gpt-4o"],"access_group_ids":[]}"#;
pub(super) const DENYING_TEAM: &str =
    r#"{"team_id":"team-1","models":["other-model"],"access_group_ids":[]}"#;
pub(super) const ALLOWING_MEMBER: &str = r#"{"user_id":"caller-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#;
pub(super) const DENYING_MEMBER: &str = r#"{"user_id":"caller-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["other-model"]}}"#;

static UNRESOLVED: UnresolvedProviderInference = UnresolvedProviderInference;
static EMPTY_ALIASES: BTreeMap<String, String> = BTreeMap::new();
static NO_SPEND: NoSpend = NoSpend;
static NO_DEFAULTS: NoDefaults = NoDefaults;
static NO_DURATIONS: NoDurations = NoDurations;

pub(super) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fixture parses")
}

pub(super) fn token(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = json(fields) else {
        panic!("the key fixture must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the key fixture constructs")
}

pub(super) fn agent_token(fields: &str) -> UserAPIKeyAuth {
    let mut built = token(fields);
    built.extra.agent_caller = Some(AgentCaller {
        user_id: Some("caller-1".to_owned()),
        team_id: Some("team-1".to_owned()),
    });
    built
}

pub(super) fn context() -> ModelAccessContext<'static> {
    context_with_depth(100)
}

pub(super) fn shallow_context() -> ModelAccessContext<'static> {
    context_with_depth(0)
}

fn context_with_depth(max_recurse_depth: i64) -> ModelAccessContext<'static> {
    ModelAccessContext {
        view: None,
        global_model_aliases: &EMPTY_ALIASES,
        compaction: None,
        provider: &UNRESOLVED,
        max_recurse_depth,
    }
}

pub(super) fn deps() -> ResolvedDeps<'static> {
    ResolvedDeps {
        teams: None,
        projects: None,
        admission: None,
        now: 0.0_f64,
    }
}

pub(super) fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

pub(super) fn ceiling(models: &[&str]) -> AgentAccessGroupCeiling {
    AgentAccessGroupCeiling {
        access_group_ids: vec!["group-1".to_owned()],
        models: models.iter().map(|model| (*model).to_owned()).collect(),
        mcp_server_ids: BTreeSet::new(),
        agent_ids: BTreeSet::new(),
    }
}

fn access_groups_failure() -> AccessGroupsError {
    AccessGroupsError::Read {
        access_group_id: "group-1".to_owned(),
        source: Box::new(TeamLookupError::NoDatabase),
    }
}

fn looked_up_team(text: &str) -> LiteLLMTeamTableCachedObj {
    LiteLLMTeamTableCachedObj::try_from(json(text)).expect("the team fixture decodes")
}

fn looked_up_user(text: &str) -> LiteLLMUserTable {
    serde_json::from_str(text).expect("the user fixture decodes")
}

struct NoSpend;

#[async_trait]
impl SpendReader for NoSpend {
    async fn read_spend(&self, query: &SpendQuery<'_>) -> Result<Decimal, SpendReadError> {
        Err(SpendReadError::Unverifiable {
            counter_key: query.counter_key.to_owned(),
        })
    }
}

struct NoDefaults;

#[async_trait]
impl DefaultBudgetSource for NoDefaults {
    async fn team_member_default(
        &self,
        _budget_id: &str,
    ) -> Result<Option<LiteLLMBudgetTable>, MemberBudgetError> {
        Ok(None)
    }
}

struct NoDurations;

impl DurationParser for NoDurations {
    fn seconds(&self, _spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        None
    }
}

#[derive(Default)]
pub(super) struct CaseMemberships {
    membership: Option<String>,
    asked: Mutex<Vec<String>>,
}

impl CaseMemberships {
    pub(super) fn allowed(models: &str) -> Self {
        Self {
            membership: Some(models.to_owned()).filter(|models| !models.is_empty()),
            asked: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl MembershipSource for CaseMemberships {
    async fn team_membership(
        &self,
        user_id: &str,
        team_id: &str,
    ) -> Result<Option<LiteLLMTeamMembership>, MembershipLoadError> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{user_id}:{team_id}"));
        Ok(self
            .membership
            .as_ref()
            .map(|text| serde_json::from_str(text).expect("the membership fixture decodes")))
    }
}

pub(super) fn admission<'deps>(
    memberships: &'deps CaseMemberships,
    models: &'deps ModelAccessContext<'deps>,
) -> TeamAdmission<'deps> {
    TeamAdmission {
        reader: &NO_SPEND,
        memberships,
        defaults: &NO_DEFAULTS,
        member_alerts: None,
        team_alerts: None,
        durations: &NO_DURATIONS,
        models,
        now: Utc::now(),
    }
}

// the port the state installs, scripted per step so every collaborator's answer is explicit
pub(super) struct Script {
    pub(super) policy: Option<String>,
    pub(super) managed: Vec<AgentAccessGroupCeiling>,
    pub(super) managed_fails: bool,
    pub(super) unmanaged: Option<AgentAccessGroupCeiling>,
    pub(super) unmanaged_fails: bool,
    pub(super) caller: Option<String>,
    pub(super) caller_fails: bool,
    pub(super) team: Option<String>,
    pub(super) team_fails: bool,
    pub(super) user: Option<String>,
    pub(super) user_fails: bool,
    pub(super) events: Mutex<Vec<String>>,
}

impl Script {
    fn record(&self, event: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    pub(super) fn events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn lookup_failure() -> AgentCallerError {
        AgentCallerError::Record(AuthError::NonStringApiKey)
    }

    fn lookup(&self, slot: CallerSlot) -> Result<Option<&str>, AgentCallerError> {
        self.record(slot.event().to_owned());
        if self.slot_fails(slot) {
            return Err(Script::lookup_failure());
        }
        Ok(self.slot_text(slot))
    }

    fn slot_fails(&self, slot: CallerSlot) -> bool {
        match slot {
            CallerSlot::Auth => self.caller_fails,
            CallerSlot::Team => self.team_fails,
            CallerSlot::User => self.user_fails,
        }
    }

    fn slot_text(&self, slot: CallerSlot) -> Option<&str> {
        match slot {
            CallerSlot::Auth => self.caller.as_deref(),
            CallerSlot::Team => self.team.as_deref(),
            CallerSlot::User => self.user.as_deref(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum CallerSlot {
    Auth,
    Team,
    User,
}

impl CallerSlot {
    fn event(self) -> &'static str {
        match self {
            Self::Auth => "caller_auth",
            Self::Team => "caller_team",
            Self::User => "caller_user",
        }
    }
}

#[async_trait]
impl AgentAccessSource for Script {
    fn managed_policy(&self, _auth: Option<&UserAPIKeyAuth>) -> Option<AgentResponse> {
        self.record("managed_policy".to_owned());
        self.policy
            .as_ref()
            .map(|text| serde_json::from_str(text).expect("the policy fixture decodes"))
    }

    async fn managed_ceilings(
        &self,
        _policy: &AgentResponse,
    ) -> Result<Vec<AgentAccessGroupCeiling>, AccessGroupsError> {
        self.record("managed_ceilings".to_owned());
        if self.managed_fails {
            return Err(access_groups_failure());
        }
        Ok(self.managed.clone())
    }

    async fn unmanaged_ceiling(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentAccessGroupCeiling>, AccessGroupsError> {
        self.record(format!("unmanaged_ceiling:{agent_id}"));
        if self.unmanaged_fails {
            return Err(access_groups_failure());
        }
        Ok(self.unmanaged.clone())
    }

    async fn caller_auth(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<UserAPIKeyAuth>, AgentCallerError> {
        Ok(self.lookup(CallerSlot::Auth)?.map(token))
    }

    async fn caller_team(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, AgentCallerError> {
        Ok(self.lookup(CallerSlot::Team)?.map(looked_up_team))
    }

    async fn caller_user(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMUserTable>, AgentCallerError> {
        Ok(self.lookup(CallerSlot::User)?.map(looked_up_user))
    }
}

pub(super) fn denial_of(error: AgentModelGrantsError, expected_type: &str) -> String {
    let AgentModelGrantsError::Denied(denial) = error else {
        panic!("the check must deny");
    };
    let proxy = denial.as_proxy_exception();
    assert_eq!(proxy.error_type, expected_type);
    assert_eq!(proxy.param.as_deref(), Some("model"));
    assert_eq!(proxy.code, "403");
    assert!(!proxy.message.contains("access groups"));
    denial.sanitized_internal_message()
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

#[derive(Clone, Default)]
struct WarningLines(Arc<Mutex<Vec<String>>>);

struct MessageField(Option<String>);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for WarningLines {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut message = MessageField(None);
        event.record(&mut message);
        if let Some(text) = message
            .0
            .filter(|_| *event.metadata().level() == Level::WARN)
        {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(text);
        }
    }
}

pub(super) fn captured<F>(runtime: &Runtime, future: F) -> (bool, Vec<String>)
where
    F: Future<Output = bool>,
{
    let lines = WarningLines::default();
    let subscriber = tracing_subscriber::registry().with(lines.clone());
    let allowed = with_default(subscriber, || runtime.block_on(future));
    let warnings = lines
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (allowed, warnings)
}

#[test]
fn warning_capture_sees_the_warn_level() {
    let runtime = runtime();
    let (_, warnings) = captured(&runtime, async {
        tracing::warn!("line");
        false
    });
    assert_eq!(warnings, ["line"]);
}
