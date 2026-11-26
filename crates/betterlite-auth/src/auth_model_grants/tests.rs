#![cfg(test)]

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
use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::denial::model_denied_client_message;
use crate::auth_model_patterns::pattern::UnresolvedProviderInference;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::error::TeamLookupError;
use crate::budget_primitives::window_start::DurationParser;
use crate::current_spend::SpendQuery;
use crate::entity_model_access::error::EntityModelAccessError;
use crate::entity_model_access::resolved::ResolvedDeps;
use crate::membership_load::error::MembershipLoadError;
use crate::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use crate::team_budget_admission::TeamAdmission;
use crate::team_budget_admission::sources::{DefaultBudgetSource, MembershipSource};

use super::access_source::AgentAccessSource;
use super::caller::check_agent_caller_model_access;
use super::ceiling::check_agent_access_group_model_access;
use super::error::AgentModelGrantsError;
use super::fallback::{RouterFallbackAccessCheck, is_model_authorized_for_token};
use super::source_adapter::{token_in_metadata, user_api_key_auth_from_request};

const AGENT_KEY: &str = r#"{"token":"sk-agent","agent_id":"agent-1"}"#;
const AGENT_TEAM_KEY: &str = r#"{"token":"sk-agent","agent_id":"agent-1","team_id":"team-1"}"#;
const AGENT_ALIAS_KEY: &str = r#"{"token":"sk-agent","agent_id":"agent-1","team_id":"team-1","team_model_aliases":{"alias":"gpt-4o"}}"#;
const AGENT_MODEL_ALIAS_KEY: &str =
    r#"{"token":"sk-agent","agent_id":"agent-1","aliases":{"fast":"gpt-4o"}}"#;
const CALLER_KEY: &str = r#"{"token":"sk-caller","user_id":"caller-1","team_id":"team-1"}"#;
const POLICY: &str = r#"{"agent_id":"agent-1","agent_name":"Agent","agent_card_params":{}}"#;
const ALLOWING_TEAM: &str = r#"{"team_id":"team-1","models":["gpt-4o"],"access_group_ids":[]}"#;
const DENYING_TEAM: &str = r#"{"team_id":"team-1","models":["other-model"],"access_group_ids":[]}"#;
const ALLOWING_MEMBER: &str = r#"{"user_id":"caller-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#;
const DENYING_MEMBER: &str = r#"{"user_id":"caller-1","team_id":"team-1","litellm_budget_table":{"allowed_models":["other-model"]}}"#;

static UNRESOLVED: UnresolvedProviderInference = UnresolvedProviderInference;
static EMPTY_ALIASES: BTreeMap<String, String> = BTreeMap::new();
static NO_SPEND: NoSpend = NoSpend;
static NO_DEFAULTS: NoDefaults = NoDefaults;
static NO_DURATIONS: NoDurations = NoDurations;

fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fixture parses")
}

fn token(fields: &str) -> UserAPIKeyAuth {
    let JsonData::Object(map) = json(fields) else {
        panic!("the key fixture must be an object");
    };
    UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the key fixture constructs")
}

fn agent_token(fields: &str) -> UserAPIKeyAuth {
    let mut built = token(fields);
    built.extra.agent_caller = Some(AgentCaller {
        user_id: Some("caller-1".to_owned()),
        team_id: Some("team-1".to_owned()),
    });
    built
}

fn context() -> ModelAccessContext<'static> {
    ModelAccessContext {
        view: None,
        global_model_aliases: &EMPTY_ALIASES,
        compaction: None,
        provider: &UNRESOLVED,
        max_recurse_depth: 100,
    }
}

fn shallow_context() -> ModelAccessContext<'static> {
    ModelAccessContext {
        view: None,
        global_model_aliases: &EMPTY_ALIASES,
        compaction: None,
        provider: &UNRESOLVED,
        max_recurse_depth: 0,
    }
}

fn deps() -> ResolvedDeps<'static> {
    ResolvedDeps {
        teams: None,
        projects: None,
        admission: None,
        now: 0.0_f64,
    }
}

fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

fn ceiling(models: &[&str]) -> AgentAccessGroupCeiling {
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
struct CaseMemberships {
    membership: Option<String>,
    asked: Mutex<Vec<String>>,
}

impl CaseMemberships {
    fn allowed(models: &str) -> Self {
        let membership = if models.is_empty() {
            None
        } else {
            Some(models.to_owned())
        };
        Self {
            membership,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<String> {
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

fn admission<'deps>(
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
#[derive(Default)]
struct Script {
    policy: Option<String>,
    managed: Vec<AgentAccessGroupCeiling>,
    managed_fails: bool,
    unmanaged: Option<AgentAccessGroupCeiling>,
    unmanaged_fails: bool,
    caller: Option<String>,
    caller_fails: bool,
    team: Option<String>,
    team_fails: bool,
    user: Option<String>,
    user_fails: bool,
    events: Mutex<Vec<String>>,
}

impl Script {
    fn record(&self, event: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn lookup_failure() -> AgentCallerError {
        AgentCallerError::Record(AuthError::NonStringApiKey)
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
        self.record("caller_auth".to_owned());
        if self.caller_fails {
            return Err(Script::lookup_failure());
        }
        Ok(self.caller.as_ref().map(|text| token(text)))
    }

    async fn caller_team(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMTeamTableCachedObj>, AgentCallerError> {
        self.record("caller_team".to_owned());
        if self.team_fails {
            return Err(Script::lookup_failure());
        }
        Ok(self.team.as_ref().map(|text| looked_up_team(text)))
    }

    async fn caller_user(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Option<LiteLLMUserTable>, AgentCallerError> {
        self.record("caller_user".to_owned());
        if self.user_fails {
            return Err(Script::lookup_failure());
        }
        Ok(self.user.as_ref().map(|text| looked_up_user(text)))
    }
}

fn denial_of(error: AgentModelGrantsError, expected_type: &str) -> String {
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

#[tokio::test]
async fn missing_model_token_agent_skips() {
    let script = Script::default();
    let context = context();
    let agent = token(AGENT_KEY);
    // a falsey model, a missing token or a missing/empty agent id returns before any lookup
    for model in [
        RequestedModels::None,
        RequestedModels::One(String::new()),
        RequestedModels::Many(Vec::new()),
    ] {
        check_agent_access_group_model_access(&model, Some(&agent), Some(&script), &context)
            .await
            .unwrap();
    }
    check_agent_access_group_model_access(&one("gpt-4o"), None, Some(&script), &context)
        .await
        .unwrap();
    for fields in [r#"{"token":"sk-plain"}"#, r#"{"token":"sk","agent_id":""}"#] {
        check_agent_access_group_model_access(
            &one("gpt-4o"),
            Some(&token(fields)),
            Some(&script),
            &context,
        )
        .await
        .unwrap();
    }
    // without a source a falsey model still skips rather than failing
    check_agent_access_group_model_access(&RequestedModels::None, Some(&agent), None, &context)
        .await
        .unwrap();
    assert!(script.events().is_empty());
}

#[tokio::test]
async fn missing_source_fails_agent_requests() {
    let context = context();
    let agent = token(AGENT_KEY);
    assert!(matches!(
        check_agent_access_group_model_access(&one("gpt-4o"), Some(&agent), None, &context).await,
        Err(AgentModelGrantsError::Unavailable)
    ));
}

#[tokio::test]
async fn managed_beats_unmanaged() {
    // the unmanaged ceiling would deny every model, so an allowed request proves it was never read
    let script = Script {
        policy: Some(POLICY.to_owned()),
        managed: vec![ceiling(&["gpt-4o"])],
        unmanaged: Some(ceiling(&[])),
        ..Script::default()
    };
    check_agent_access_group_model_access(
        &one("gpt-4o"),
        Some(&token(AGENT_KEY)),
        Some(&script),
        &context(),
    )
    .await
    .unwrap();
    assert_eq!(script.events(), ["managed_policy", "managed_ceilings"]);
}

#[tokio::test]
async fn unmanaged_none_unrestricted() {
    let script = Script::default();
    check_agent_access_group_model_access(
        &one("private-model"),
        Some(&token(AGENT_KEY)),
        Some(&script),
        &context(),
    )
    .await
    .unwrap();
    assert_eq!(
        script.events(),
        ["managed_policy", "unmanaged_ceiling:agent-1"]
    );
}

#[tokio::test]
async fn empty_ceiling_denies_403() {
    let script = Script {
        policy: Some(POLICY.to_owned()),
        managed: vec![ceiling(&[])],
        ..Script::default()
    };
    let error = check_agent_access_group_model_access(
        &one("secret-model"),
        Some(&token(AGENT_KEY)),
        Some(&script),
        &context(),
    )
    .await
    .unwrap_err();
    let internal = denial_of(error, "agent_model_access_denied");
    assert_eq!(internal, "agent agent-1 access groups grant no models");
    let AgentModelGrantsError::Denied(denial) = check_agent_access_group_model_access(
        &one("secret-model"),
        Some(&token(AGENT_KEY)),
        Some(&script),
        &context(),
    )
    .await
    .unwrap_err() else {
        panic!("the check must deny");
    };
    assert_eq!(
        denial.as_proxy_exception().message,
        model_denied_client_message("secret-model")
    );
}

#[tokio::test]
async fn all_ceilings_intersect() {
    let script = Script {
        policy: Some(POLICY.to_owned()),
        managed: vec![ceiling(&["gpt-4o"]), ceiling(&["other-model"])],
        ..Script::default()
    };
    // the first ceiling allows, so the denial can only come from the second one
    let error = check_agent_access_group_model_access(
        &one("gpt-4o"),
        Some(&token(AGENT_KEY)),
        Some(&script),
        &context(),
    )
    .await
    .unwrap_err();
    let internal = denial_of(error, "agent_model_access_denied");
    assert!(internal.contains("models=[\"other-model\"]"), "{internal}");

    let both_allow = Script {
        policy: Some(POLICY.to_owned()),
        managed: vec![ceiling(&["gpt-4o"]), ceiling(&["gpt-4o", "other-model"])],
        ..Script::default()
    };
    check_agent_access_group_model_access(
        &one("gpt-4o"),
        Some(&token(AGENT_KEY)),
        Some(&both_allow),
        &context(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn team_alias_before_ceiling() {
    let script = Script {
        policy: Some(POLICY.to_owned()),
        managed: vec![ceiling(&["gpt-4o"])],
        ..Script::default()
    };
    // the alias resolves to gpt-4o before the ceiling sees the name
    check_agent_access_group_model_access(
        &one("alias"),
        Some(&token(AGENT_ALIAS_KEY)),
        Some(&script),
        &context(),
    )
    .await
    .unwrap();
    // without the alias map the same request denies, so the resolution is what allowed it
    assert!(matches!(
        check_agent_access_group_model_access(
            &one("alias"),
            Some(&token(AGENT_TEAM_KEY)),
            Some(&script),
            &context(),
        )
        .await,
        Err(AgentModelGrantsError::Denied(_))
    ));
}

#[tokio::test]
async fn sorted_models_and_key_aliases() {
    let sorted = Script {
        policy: Some(POLICY.to_owned()),
        managed: vec![ceiling(&["z-model", "a-model"])],
        ..Script::default()
    };
    let error = check_agent_access_group_model_access(
        &one("missing-model"),
        Some(&token(AGENT_KEY)),
        Some(&sorted),
        &context(),
    )
    .await
    .unwrap_err();
    let internal = denial_of(error, "agent_model_access_denied");
    assert!(
        internal.contains("models=[\"a-model\", \"z-model\"]"),
        "{internal}"
    );

    let aliased = Script {
        policy: Some(POLICY.to_owned()),
        managed: vec![ceiling(&["gpt-4o"])],
        ..Script::default()
    };
    // the ceiling holds only the target; the key alias rewrites the request onto it
    check_agent_access_group_model_access(
        &one("fast"),
        Some(&token(AGENT_MODEL_ALIAS_KEY)),
        Some(&aliased),
        &context(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn ceiling_lookup_error_propagates() {
    let managed = Script {
        policy: Some(POLICY.to_owned()),
        managed_fails: true,
        ..Script::default()
    };
    assert!(matches!(
        check_agent_access_group_model_access(
            &one("gpt-4o"),
            Some(&token(AGENT_KEY)),
            Some(&managed),
            &context(),
        )
        .await,
        Err(AgentModelGrantsError::Ceiling(_))
    ));
    let unmanaged = Script {
        unmanaged_fails: true,
        ..Script::default()
    };
    assert!(matches!(
        check_agent_access_group_model_access(
            &one("gpt-4o"),
            Some(&token(AGENT_KEY)),
            Some(&unmanaged),
            &context(),
        )
        .await,
        Err(AgentModelGrantsError::Ceiling(_))
    ));
}

#[tokio::test]
async fn ceiling_recurse_depth_propagates() {
    let script = Script {
        policy: Some(POLICY.to_owned()),
        managed: vec![ceiling(&["gpt-4o"])],
        ..Script::default()
    };
    assert!(matches!(
        check_agent_access_group_model_access(
            &one("gpt-4o"),
            Some(&token(AGENT_KEY)),
            Some(&script),
            &shallow_context(),
        )
        .await,
        Err(AgentModelGrantsError::RecurseDepth { model }) if model == "gpt-4o"
    ));
}

#[tokio::test]
async fn caller_absent_skips() {
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(DENYING_TEAM.to_owned()),
        ..Script::default()
    };
    let memberships = CaseMemberships::default();
    let context = context();
    let admission = admission(&memberships, &context);
    check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&token(AGENT_KEY)),
        Some(&script),
        &admission,
        &context,
    )
    .await
    .unwrap();
    assert!(script.events().is_empty());
    assert!(memberships.asked().is_empty());
}

#[tokio::test]
async fn caller_team_before_membership() {
    let context = context();
    // the team grant allows, then the member scope decides
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(ALLOWING_TEAM.to_owned()),
        ..Script::default()
    };
    let memberships = CaseMemberships::allowed(ALLOWING_MEMBER);
    let admission = admission(&memberships, &context);
    check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &admission,
        &context,
    )
    .await
    .unwrap();
    assert_eq!(script.events(), ["caller_auth", "caller_team"]);
    assert_eq!(memberships.asked(), ["caller-1:team-1"]);

    // a team denial ends the check without asking the member source or the user
    let denying = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(DENYING_TEAM.to_owned()),
        ..Script::default()
    };
    let memberships = CaseMemberships::allowed(ALLOWING_MEMBER);
    let admission = admission(&memberships, &context);
    let error = check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&denying),
        &admission,
        &context,
    )
    .await
    .unwrap_err();
    let AgentModelGrantsError::Entity(boxed) = error else {
        panic!("the caller team denies");
    };
    let EntityModelAccessError::Denied(denial) = *boxed else {
        panic!("the caller team denial is a model access denial");
    };
    assert_eq!(
        denial.as_proxy_exception().error_type,
        "team_model_access_denied"
    );
    assert!(memberships.asked().is_empty());
    assert!(!denying.events().contains(&"caller_user".to_owned()));

    // the member scope denies when the team grant passes but the member allowlist does not
    let memberships = CaseMemberships::allowed(DENYING_MEMBER);
    let admission = admission(&memberships, &context);
    let error = check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&Script {
            caller: Some(CALLER_KEY.to_owned()),
            team: Some(ALLOWING_TEAM.to_owned()),
            ..Script::default()
        }),
        &admission,
        &context,
    )
    .await
    .unwrap_err();
    let AgentModelGrantsError::Entity(boxed) = error else {
        panic!("the member scope denies");
    };
    let EntityModelAccessError::Denied(denial) = *boxed else {
        panic!("the member denial is a model access denial");
    };
    assert_eq!(
        denial.as_proxy_exception().error_type,
        "team_model_access_denied"
    );
    assert_eq!(memberships.asked(), ["caller-1:team-1"]);
}

#[tokio::test]
async fn caller_team_skips_user() {
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(ALLOWING_TEAM.to_owned()),
        user: Some(r#"{"user_id":"caller-1","models":["other-model"]}"#.to_owned()),
        ..Script::default()
    };
    let memberships = CaseMemberships::allowed(ALLOWING_MEMBER);
    let context = context();
    let admission = admission(&memberships, &context);
    check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &admission,
        &context,
    )
    .await
    .unwrap();
    assert_eq!(script.events(), ["caller_auth", "caller_team"]);
}

#[tokio::test]
async fn caller_user_fallback() {
    let context = context();
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        user: Some(r#"{"user_id":"caller-1","models":["gpt-4o"]}"#.to_owned()),
        ..Script::default()
    };
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &admission,
        &context,
    )
    .await
    .unwrap();
    assert_eq!(
        script.events(),
        ["caller_auth", "caller_team", "caller_user"]
    );

    let denying = Script {
        caller: Some(CALLER_KEY.to_owned()),
        user: Some(r#"{"user_id":"caller-1","models":["other-model"]}"#.to_owned()),
        ..Script::default()
    };
    let error = check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&denying),
        &admission,
        &context,
    )
    .await
    .unwrap_err();
    let AgentModelGrantsError::Entity(boxed) = error else {
        panic!("the caller user denies");
    };
    let EntityModelAccessError::Denied(denial) = *boxed else {
        panic!("the caller user denial is a model access denial");
    };
    assert_eq!(
        denial.as_proxy_exception().error_type,
        "user_model_access_denied"
    );
}

#[tokio::test]
async fn missing_caller_user_skips() {
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        ..Script::default()
    };
    let memberships = CaseMemberships::default();
    let context = context();
    let admission = admission(&memberships, &context);
    check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &admission,
        &context,
    )
    .await
    .unwrap();
    assert!(script.events().contains(&"caller_user".to_owned()));
}

#[tokio::test]
async fn caller_lookup_error_propagates() {
    let context = context();
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    for script in [
        Script {
            caller_fails: true,
            ..Script::default()
        },
        Script {
            caller: Some(CALLER_KEY.to_owned()),
            team_fails: true,
            ..Script::default()
        },
        Script {
            caller: Some(CALLER_KEY.to_owned()),
            user_fails: true,
            ..Script::default()
        },
    ] {
        let error = check_agent_caller_model_access(
            &one("gpt-4o"),
            Some(&agent_token(AGENT_KEY)),
            Some(&script),
            &admission,
            &context,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AgentModelGrantsError::Caller(_)));
    }
}

#[tokio::test]
async fn missing_source_fails_callers() {
    let context = context();
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    assert!(matches!(
        check_agent_caller_model_access(
            &one("gpt-4o"),
            Some(&agent_token(AGENT_KEY)),
            None,
            &admission,
            &context,
        )
        .await,
        Err(AgentModelGrantsError::Unavailable)
    ));
}

#[test]
fn disabled_skips_token() {
    let runtime = runtime();
    let token = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    let check = RouterFallbackAccessCheck {
        is_enforced: false,
        token: Some(&token),
        context: &context,
        deps: &deps,
    };
    let (allowed, warnings) = captured(&runtime, check.call("other-model"));
    assert!(allowed);
    assert!(warnings.is_empty());
}

#[tokio::test]
async fn enabled_without_token_allows() {
    let context = context();
    let deps = deps();
    let check = RouterFallbackAccessCheck {
        is_enforced: true,
        token: None,
        context: &context,
        deps: &deps,
    };
    assert!(check.call("anything").await);
}

#[tokio::test]
async fn metadata_before_litellm_metadata() {
    let mut request = BTreeMap::new();
    request.insert(
        "metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-first","team_id":"team-first"}}"#),
    );
    request.insert(
        "litellm_metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-second","team_id":"team-second"}}"#),
    );
    let resolved =
        user_api_key_auth_from_request(&request).expect("the first metadata holds a key");
    assert_eq!(resolved.view.token.team_id.as_deref(), Some("team-first"));

    // metadata without a token falls through to litellm_metadata
    let mut fallthrough = BTreeMap::new();
    fallthrough.insert("metadata".to_owned(), json(r#"{"user_api_key_auth":null}"#));
    fallthrough.insert(
        "litellm_metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-second","team_id":"team-second"}}"#),
    );
    let resolved =
        user_api_key_auth_from_request(&fallthrough).expect("the second metadata holds a key");
    assert_eq!(resolved.view.token.team_id.as_deref(), Some("team-second"));
}

#[tokio::test]
async fn invalid_metadata_tries_second() {
    let mut request = BTreeMap::new();
    // a shape Pydantic rejects is skipped, not fatal
    request.insert(
        "metadata".to_owned(),
        json(r#"{"user_api_key_auth":"not-an-object"}"#),
    );
    request.insert(
        "litellm_metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-second","team_id":"team-second"}}"#),
    );
    let resolved = user_api_key_auth_from_request(&request).expect("the valid metadata wins");
    assert_eq!(resolved.view.token.team_id.as_deref(), Some("team-second"));
    assert!(token_in_metadata(&json("\"text\"")).is_none());
    assert!(token_in_metadata(&json(r#"{"user_api_key_auth":[]}"#)).is_none());
    assert!(token_in_metadata(&json(r#"{"other":{}}"#)).is_none());
}

#[tokio::test]
async fn resolved_policy_without_model_list() {
    // the key's own models decide; the check receives no router model list
    let token =
        token(r#"{"token":"sk","models":["gpt-4o"],"team_id":"team-1","team_models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    assert!(is_model_authorized_for_token("gpt-4o", &token, &context, &deps).await);
    assert!(!is_model_authorized_for_token("other-model", &token, &context, &deps).await);
}

#[test]
fn proxy_denial_silent_false() {
    let runtime = runtime();
    let token = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    let (allowed, warnings) = captured(
        &runtime,
        is_model_authorized_for_token("other-model", &token, &context, &deps),
    );
    assert!(!allowed);
    assert!(warnings.is_empty());
}

#[test]
fn lookup_failure_warns_false() {
    let runtime = runtime();
    let token = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = shallow_context();
    let deps = deps();
    let (allowed, warnings) = captured(
        &runtime,
        is_model_authorized_for_token("gpt-4o", &token, &context, &deps),
    );
    assert!(!allowed);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with("Skipping fallback to model=gpt-4o: authorization lookup failed:"),
        "{warnings:?}"
    );
}

#[tokio::test]
async fn fallback_success_true() {
    let token = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    assert!(is_model_authorized_for_token("gpt-4o", &token, &context, &deps).await);
}

#[tokio::test]
async fn fallback_uses_auth_context() {
    let denying = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let context = context();
    let deps = deps();
    let check = RouterFallbackAccessCheck {
        is_enforced: true,
        token: Some(&denying),
        context: &context,
        deps: &deps,
    };
    assert!(!check.call("secret-model").await);
    assert!(check.call("gpt-4o").await);
}

#[tokio::test]
async fn forged_metadata_cannot_grant() {
    let denying = token(r#"{"token":"sk","models":["gpt-4o"]}"#);
    let mut forged = BTreeMap::new();
    forged.insert(
        "metadata".to_owned(),
        json(r#"{"user_api_key_auth":{"token":"sk-forged","models":["secret-model"]}}"#),
    );
    // the comparison adapter decodes the forged token, and production still uses the authenticated one
    let decoded = user_api_key_auth_from_request(&forged).expect("the forged shape decodes");
    assert_eq!(decoded.view.token.models, vec![json("\"secret-model\"")]);
    let context = context();
    let deps = deps();
    let check = RouterFallbackAccessCheck {
        is_enforced: true,
        token: Some(&denying),
        context: &context,
        deps: &deps,
    };
    assert!(!check.call("secret-model").await);
}

fn runtime() -> Runtime {
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

fn captured(runtime: &Runtime, future: impl Future<Output = bool>) -> (bool, Vec<String>) {
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
