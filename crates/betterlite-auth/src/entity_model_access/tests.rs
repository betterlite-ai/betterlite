#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_db::team_rows::team::LiteLLMTeamTable;
use betterlite_error::proxy_error_types::ProxyErrorTypes;
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;
use chrono::Utc;
use rstest::rstest;

use crate::auth_model_patterns::access_groups::{AccessGroupError, AccessGroupLookup as _};
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::lookup::TeamLookup;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::error::EntityModelAccessError;
use super::groups::{
    GroupResources, ResourceKind, authorized_resources, key_access_group_grants_model,
};
use super::key::can_key_call_model;
use super::org::can_org_access_model;
use super::project::can_project_access_model;
use super::resolved::{ResolvedDeps, can_key_call_resolved_model};
use super::team::{TeamAccessRequest, can_team_access_model};

mod support;

use support::{
    Script, Setup, admission, group_row, one, org_row, project_lookup, project_row, team_lookup,
    team_row, token,
};

const TEAM_ONE: &str = r#"{"team_id":"team-1","team_alias":"Team One","models":["gpt-4o"],"access_group_ids":["ag-team"]}"#;
const TEAM_TWO: &str = r#"{"team_id":"team-2","models":["gpt-4o"]}"#;
const PROJECT_ALLOWING: &str = r#"{"project_id":"project-1","models":["gpt-4o"]}"#;
const PROJECT_DENYING: &str = r#"{"project_id":"project-1","models":["claude"]}"#;
const ORG_ALLOWING: &str = r#"{"organization_id":"org-1","budget_id":"budget-1","created_by":"creator","updated_by":"editor","models":["gpt-4o"]}"#;
const GROUP_KEY: &str = r#"{"access_group_id":"ag-key","access_group_name":"key group","access_model_names":["claude"]}"#;
const GROUP_EMPTY: &str = r#"{"access_group_id":"ag-empty","access_group_name":"empty group"}"#;
const GROUP_TEAM: &str = r#"{"access_group_id":"ag-team","access_group_name":"team group","access_model_names":["claude","gpt-4o"]}"#;
const GROUP_OWNER: &str = r#"{"access_group_id":"ag-owner","access_group_name":"owner group","access_model_names":["claude"],"assigned_team_ids":["team-1"],"assigned_key_ids":["hash-1"]}"#;
const GROUP_OTHER_TEAM: &str = r#"{"access_group_id":"ag-other","access_group_name":"other group","access_model_names":["claude"],"assigned_team_ids":["team-2"]}"#;
const GROUP_KEY_OWNED: &str = r#"{"access_group_id":"ag-by-key","access_group_name":"key owned","access_model_names":["claude"],"assigned_key_ids":["hash-1"]}"#;
const GROUP_MATRIX_ONE: &str = r#"{"access_group_id":"ag-one","access_group_name":"one","access_model_names":["gpt-4o","claude"],"access_mcp_server_ids":["mcp-1"],"access_agent_ids":["agent-1"],"assigned_team_ids":["team-1"]}"#;
const GROUP_MATRIX_TWO: &str = r#"{"access_group_id":"ag-two","access_group_name":"two","access_model_names":["claude"],"access_mcp_server_ids":["mcp-1","mcp-2"],"access_agent_ids":["agent-2"],"assigned_team_ids":["team-1"]}"#;
const TOKEN_PLAIN: &str = r#"{"token":"hash-1","models":["gpt-4o"]}"#;
const TOKEN_GROUPS: &str =
    r#"{"token":"hash-1","models":["gpt-4o"],"access_group_ids":["ag-key"]}"#;
const TOKEN_EMPTY_GROUP: &str =
    r#"{"token":"hash-1","models":["gpt-4o"],"access_group_ids":["ag-empty"]}"#;
const TOKEN_MISSING_GROUP: &str =
    r#"{"token":"hash-1","models":["gpt-4o"],"access_group_ids":["ag-missing"]}"#;
const TOKEN_TEAM: &str = r#"{"token":"hash-1","team_id":"team-1"}"#;
const TOKEN_SNAPSHOT: &str = r#"{"token":"hash-1","team_id":"team-1","team_models":["gpt-4o"],"team_alias":"Snap","team_metadata":{"tier":"gold"},"team_object_permission_id":"perm-1"}"#;
const TOKEN_MEMBER: &str = r#"{"token":"hash-1","team_id":"team-1","user_id":"user-1"}"#;
const TOKEN_PROJECT: &str =
    r#"{"token":"hash-1","team_id":"team-1","user_id":"user-1","project_id":"project-1"}"#;
const TOKEN_CONFIGURED: &str = r#"{"token":"hash-1","models":["gpt-4o"],"config":{"rpm":1}}"#;
const TOKEN_ALL_TEAM: &str = r#"{"token":"hash-1","models":["all-team-models"]}"#;
const MEMBER_DENYING: &str = r#"{"user_id":"user-1","team_id":"team-1","budget_id":"budget-1","litellm_budget_table":{"allowed_models":["claude"]}}"#;
const MEMBER_ALLOWING: &str = r#"{"user_id":"user-1","team_id":"team-1","budget_id":"budget-1","litellm_budget_table":{"allowed_models":["gpt-4o"]}}"#;

fn denial(error: &EntityModelAccessError) -> &ModelAccessDeniedProxyException {
    error.denial().expect("the error is a model access denial")
}

fn error_type(error: &EntityModelAccessError) -> String {
    denial(error).as_proxy_exception().error_type.clone()
}

fn internal(error: &EntityModelAccessError) -> String {
    denial(error).sanitized_internal_message()
}

fn assert_denial(error: &EntityModelAccessError, kind: ProxyErrorTypes) {
    assert_eq!(error_type(error), kind.to_string());
    let exception = denial(error).as_proxy_exception();
    assert_eq!(exception.param.as_deref(), Some("model"));
    assert_eq!(exception.code, "403");
    assert!(!exception.message.contains("models="));
}

const fn empty_aliases() -> BTreeMap<String, String> {
    BTreeMap::new()
}

const fn team_access<'request>(
    team: Option<&'request LiteLLMTeamTable>,
    aliases: &'request BTreeMap<String, String>,
    lookup: Option<&'request TeamLookup>,
) -> TeamAccessRequest<'request> {
    TeamAccessRequest {
        team,
        team_aliases: aliases,
        key_aliases: aliases,
        lookup,
    }
}

async fn resolved_with(
    script: &Arc<Script>,
    valid_token: &UserAPIKeyAuth,
    model: &RequestedModels,
) -> Result<(), EntityModelAccessError> {
    let setup = Setup::new();
    let context = setup.context();
    let teams = team_lookup(script);
    let projects = project_lookup(script);
    let team_admission = admission(script, &context, Utc::now());
    let deps = ResolvedDeps {
        teams: Some(&teams),
        projects: Some(&projects),
        admission: Some(&team_admission),
        now: 0.0,
    };
    can_key_call_resolved_model(model, valid_token, &context, &deps).await
}

#[test]
fn org_denial() {
    let setup = Setup::new();
    let context = setup.context();
    let org = org_row(ORG_ALLOWING);
    let aliases = empty_aliases();
    let refused = can_org_access_model(&one("claude"), Some(&org), &aliases, &context).unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::OrgModelAccessDenied.to_string()
    );
    assert_eq!(
        denial(&refused).as_proxy_exception().param.as_deref(),
        Some("model")
    );
    assert_eq!(denial(&refused).as_proxy_exception().code, "403");
    // an empty allowlist is unrestricted, and so is an absent organization
    can_org_access_model(&one("gpt-4o"), Some(&org), &aliases, &context).unwrap();
    can_org_access_model(&one("claude"), None, &aliases, &context).unwrap();
}

#[tokio::test]
async fn key_resolved_target() {
    let setup = Setup::new();
    let context = setup.context();
    let aliased = token(r#"{"token":"hash-1","models":["gpt-4o"],"aliases":{"fast":"gpt-4o"}}"#);
    can_key_call_model(&one("fast"), &aliased, &context, None)
        .await
        .unwrap();
    // the requested alias rewrites to its target, which the allowlist must hold
    let alias_only = token(r#"{"token":"hash-1","models":["fast"],"aliases":{"fast":"gpt-4o"}}"#);
    let refused = can_key_call_model(&one("fast"), &alias_only, &context, None)
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
    assert!(internal(&refused).contains("models=[\"fast\"]"));
}

#[test]
fn project_model_denial() {
    let setup = Setup::new();
    let context = setup.context();
    let project = project_row(PROJECT_ALLOWING);
    let aliases = BTreeMap::from([("fast".to_owned(), "gpt-4o".to_owned())]);
    can_project_access_model(&one("fast"), Some(&project), &aliases, &context).unwrap();
    let refused =
        can_project_access_model(&one("claude"), Some(&project), &aliases, &context).unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::ProjectModelAccessDenied.to_string()
    );
    assert_eq!(
        denial(&refused).as_proxy_exception().param.as_deref(),
        Some("model")
    );
    assert_eq!(denial(&refused).as_proxy_exception().code, "403");
    can_project_access_model(&one("claude"), None, &aliases, &context).unwrap();
}

#[tokio::test]
async fn key_groups_replace_grants() {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_KEY)));
    let lookup = team_lookup(&script);
    let setup = Setup::new();
    let context = setup.context();
    let valid_token = token(TOKEN_GROUPS);
    can_key_call_model(&one("claude"), &valid_token, &context, Some(&lookup))
        .await
        .unwrap();
    can_key_call_model(&one("gpt-4o"), &valid_token, &context, Some(&lookup))
        .await
        .unwrap();
    let refused = can_key_call_model(&one("other"), &valid_token, &context, Some(&lookup))
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
    assert_eq!(internal(&refused), single_model_internal("claude", "other"));
}

fn single_model_internal(models: &str, tried: &str) -> String {
    format!(
        "key not allowed to access model. This key can only access models=[\"{models}\"]. Tried \
         to access {tried}"
    )
}

#[tokio::test]
async fn team_groups_union_preserves_order() {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_TEAM)));
    let lookup = team_lookup(&script);
    let team = team_row(TEAM_ONE);
    let setup = Setup::new();
    let context = setup.context();
    let aliases = empty_aliases();
    can_team_access_model(
        &one("claude"),
        &team_access(Some(&team), &aliases, Some(&lookup)),
        &context,
    )
    .await
    .unwrap();
    let refused = can_team_access_model(
        &one("other"),
        &team_access(Some(&team), &aliases, Some(&lookup)),
        &context,
    )
    .await
    .unwrap_err();
    // the team's own models come first and the group's follow, deduplicated
    assert!(internal(&refused).contains("models=[\"gpt-4o\", \"claude\"]"));
}

#[tokio::test]
async fn only_proxy_exception_falls_back() {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_KEY)));
    let lookup = team_lookup(&script);
    let setup = Setup {
        aliases: BTreeMap::new(),
        max_depth: 0,
    };
    let context = setup.context();
    let valid_token = token(TOKEN_GROUPS);
    let refused = can_key_call_model(&one("claude"), &valid_token, &context, Some(&lookup))
        .await
        .unwrap_err();
    // the depth failure is a bare Exception in the source, so the group fallback never runs
    assert!(matches!(
        refused,
        EntityModelAccessError::RecurseDepth { .. }
    ));
    let team = team_row(TEAM_ONE);
    let aliases = empty_aliases();
    let team_refused = can_team_access_model(
        &one("claude"),
        &team_access(Some(&team), &aliases, Some(&lookup)),
        &context,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        team_refused,
        EntityModelAccessError::RecurseDepth { .. }
    ));
    assert!(script.reads().is_empty());
}

#[rstest]
#[case::groups(TOKEN_GROUPS)]
#[case::empty_group(TOKEN_EMPTY_GROUP)]
#[case::missing_group(TOKEN_MISSING_GROUP)]
#[tokio::test]
async fn empty_groups_original_denial(#[case] text: &str) {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_EMPTY)));
    let lookup = team_lookup(&script);
    let setup = Setup::new();
    let context = setup.context();
    let refused = can_key_call_model(&one("claude"), &token(text), &context, Some(&lookup))
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
    assert!(internal(&refused).contains("models=[\"gpt-4o\"]"));
}

#[tokio::test]
async fn entity_denial_type_matrix() {
    let script = Arc::new(Script::new());
    let lookup = team_lookup(&script);
    let setup = Setup::new();
    let context = setup.context();
    let aliases = empty_aliases();
    let key_refused = can_key_call_model(&one("claude"), &token(TOKEN_PLAIN), &context, None)
        .await
        .unwrap_err();
    let team = team_row(TEAM_TWO);
    let team_refused = can_team_access_model(
        &one("claude"),
        &team_access(Some(&team), &aliases, Some(&lookup)),
        &context,
    )
    .await
    .unwrap_err();
    let org_refused = can_org_access_model(
        &one("claude"),
        Some(&org_row(ORG_ALLOWING)),
        &aliases,
        &context,
    )
    .unwrap_err();
    let project_refused = can_project_access_model(
        &one("claude"),
        Some(&project_row(PROJECT_ALLOWING)),
        &aliases,
        &context,
    )
    .unwrap_err();
    assert_denial(&key_refused, ProxyErrorTypes::KeyModelAccessDenied);
    assert_denial(&team_refused, ProxyErrorTypes::TeamModelAccessDenied);
    assert_denial(&org_refused, ProxyErrorTypes::OrgModelAccessDenied);
    assert_denial(&project_refused, ProxyErrorTypes::ProjectModelAccessDenied);
}

#[tokio::test]
async fn configured_key_skips_check() {
    let setup = Setup::new();
    let context = setup.context();
    let deps = ResolvedDeps {
        teams: None,
        projects: None,
        admission: None,
        now: 0.0,
    };
    can_key_call_resolved_model(&one("claude"), &token(TOKEN_CONFIGURED), &context, &deps)
        .await
        .unwrap();
    let refused = can_key_call_resolved_model(&one("claude"), &token(TOKEN_PLAIN), &context, &deps)
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
}

#[tokio::test]
async fn all_team_models_skips_key() {
    let setup = Setup::new();
    let context = setup.context();
    let deps = ResolvedDeps {
        teams: None,
        projects: None,
        admission: None,
        now: 0.0,
    };
    // without a team the sentinel resolves to an empty list, which is unrestricted, yet the key check is skipped
    can_key_call_resolved_model(&one("claude"), &token(TOKEN_ALL_TEAM), &context, &deps)
        .await
        .unwrap();
    let refused = can_key_call_resolved_model(&one("claude"), &token(TOKEN_PLAIN), &context, &deps)
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
    // with a team the sentinel inherits the key's team models, whose allowlist denies the request
    let script =
        Arc::new(Script::new().with_team(team_row(r#"{"team_id":"team-1","models":["claude"]}"#)));
    let lookup = team_lookup(&script);
    let live = ResolvedDeps {
        teams: Some(&lookup),
        projects: None,
        admission: None,
        now: 0.0,
    };
    let inheriting = token(
        r#"{"token":"hash-1","team_id":"team-1","team_models":["gpt-4o"],"models":["all-team-models"]}"#,
    );
    can_key_call_resolved_model(&one("claude"), &inheriting, &context, &live)
        .await
        .unwrap();
}

#[tokio::test]
async fn team_aliases_grant_their_name() {
    let script = Arc::new(Script::new().with_team(team_row(TEAM_ONE)));
    let lookup = team_lookup(&script);
    let setup = Setup::new();
    let context = setup.context();
    let deps = ResolvedDeps {
        teams: Some(&lookup),
        projects: None,
        admission: None,
        now: 0.0,
    };
    // a team alias grants the alias name itself, whatever target it names
    let aliased =
        token(r#"{"token":"hash-1","team_id":"team-1","team_model_aliases":{"fast":"claude"}}"#);
    can_key_call_resolved_model(&one("fast"), &aliased, &context, &deps)
        .await
        .unwrap();
    // a map that does not name the requested model leaves it to the team's allowlist
    let unaliased =
        token(r#"{"token":"hash-1","team_id":"team-1","team_model_aliases":{"other":"claude"}}"#);
    let refused = can_key_call_resolved_model(&one("fast"), &unaliased, &context, &deps)
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::TeamModelAccessDenied.to_string()
    );
}

#[tokio::test]
async fn resolved_default_db_argument() {
    let script = Arc::new(
        Script::new()
            .with_group(group_row(GROUP_KEY))
            .with_group(group_row(GROUP_TEAM))
            .with_team(team_row(TEAM_ONE)),
    );
    let lookup = team_lookup(&script);
    let setup = Setup::new();
    let context = setup.context();
    let deps = ResolvedDeps {
        teams: Some(&lookup),
        projects: None,
        admission: None,
        now: 0.0,
    };
    // the omitted prisma_client defaults to the proxy's own, so the key's groups expand inside this flow
    can_key_call_resolved_model(&one("claude"), &token(TOKEN_GROUPS), &context, &deps)
        .await
        .unwrap();
    // the team check gets the same default, so the team's own groups expand onto its list
    let team_token = token(r#"{"token":"hash-1","team_id":"team-1","config":{"rpm":1}}"#);
    can_key_call_resolved_model(&one("claude"), &team_token, &context, &deps)
        .await
        .unwrap();
    let reads = script.reads();
    assert!(
        reads
            .iter()
            .any(|event| event.contains("access_group_id:ag-key"))
    );
    assert!(
        reads
            .iter()
            .any(|event| event.contains("access_group_id:ag-team"))
    );
}

#[tokio::test]
async fn resolved_offline_falls_back_to_plain_grants() {
    let setup = Setup::new();
    let context = setup.context();
    let offline = ResolvedDeps {
        teams: None,
        projects: None,
        admission: None,
        now: 0.0,
    };
    // with no database both checks fall back to the plain grants and the original denial stands
    let key_refused =
        can_key_call_resolved_model(&one("claude"), &token(TOKEN_GROUPS), &context, &offline)
            .await
            .unwrap_err();
    assert_eq!(
        error_type(&key_refused),
        ProxyErrorTypes::KeyModelAccessDenied.to_string()
    );
    // without a lookup the team is rebuilt from the key, whose empty team_models grant every model
    let team_token = token(r#"{"token":"hash-1","team_id":"team-1","config":{"rpm":1}}"#);
    can_key_call_resolved_model(&one("claude"), &team_token, &context, &offline)
        .await
        .unwrap();
}

#[tokio::test]
async fn team_lookup_error_snapshot() {
    let script = Arc::new(Script::new().with_membership("user-1", "team-1", MEMBER_DENYING));
    let refused = resolved_with(&script, &token(TOKEN_SNAPSHOT), &one("claude"))
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::TeamModelAccessDenied.to_string()
    );
    assert!(internal(&refused).contains("models=[\"gpt-4o\"]"));
    // the fallback team comes from the key snapshot, and a reconstructed team never answers the member scope
    resolved_with(&script, &token(TOKEN_SNAPSHOT), &one("gpt-4o"))
        .await
        .unwrap();
    let reads = script.reads();
    assert!(reads.iter().any(|event| event.contains("team_id:team-1")));
    assert!(!reads.iter().any(|event| event.contains("read membership")));
}

#[tokio::test]
async fn only_team_denial_override() {
    let script = Arc::new(
        Script::new()
            .with_team(team_row(TEAM_ONE))
            .with_group(group_row(GROUP_OWNER))
            .with_group(group_row(GROUP_KEY)),
    );
    let lookup = team_lookup(&script);
    let setup = Setup::new();
    let context = setup.context();
    let owner = token(r#"{"token":"hash-1","team_id":"team-1","access_group_ids":["ag-owner"]}"#);
    let deps = ResolvedDeps {
        teams: Some(&lookup),
        projects: None,
        admission: None,
        now: 0.0,
    };
    can_key_call_resolved_model(&one("claude"), &owner, &context, &deps)
        .await
        .unwrap();
    // a key group that does not own the key leaves the team denial standing
    let stranger = token(r#"{"token":"hash-1","team_id":"team-1","access_group_ids":["ag-key"]}"#);
    let refused = can_key_call_resolved_model(&one("claude"), &stranger, &context, &deps)
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::TeamModelAccessDenied.to_string()
    );
}

#[tokio::test]
async fn member_check_requires_live_team() {
    let script = Arc::new(Script::new().with_team(team_row(TEAM_ONE)).with_membership(
        "user-1",
        "team-1",
        MEMBER_DENYING,
    ));
    let refused = resolved_with(&script, &token(TOKEN_MEMBER), &one("gpt-4o"))
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&refused),
        ProxyErrorTypes::TeamModelAccessDenied.to_string()
    );
    assert!(internal(&refused).contains("Team member not allowed to access model"));
    // a key without a user never reaches the member scope
    let without_user = Arc::new(Script::new().with_team(team_row(TEAM_ONE)).with_membership(
        "user-1",
        "team-1",
        MEMBER_DENYING,
    ));
    resolved_with(&without_user, &token(TOKEN_TEAM), &one("gpt-4o"))
        .await
        .unwrap();
    assert!(
        !without_user
            .reads()
            .iter()
            .any(|event| event.contains("read membership"))
    );
}

#[tokio::test]
async fn project_after_member_nonempty_only() {
    let denying = Arc::new(
        Script::new()
            .with_team(team_row(TEAM_ONE))
            .with_project(project_row(PROJECT_DENYING))
            .with_membership("user-1", "team-1", MEMBER_DENYING),
    );
    // the member scope runs before the project, so its denial is the one the caller sees
    let member_refused = resolved_with(&denying, &token(TOKEN_PROJECT), &one("gpt-4o"))
        .await
        .unwrap_err();
    assert!(internal(&member_refused).contains("Team member not allowed to access model"));
    let allowing = Arc::new(
        Script::new()
            .with_team(team_row(TEAM_ONE))
            .with_project(project_row(PROJECT_DENYING))
            .with_membership("user-1", "team-1", MEMBER_ALLOWING),
    );
    let project_refused = resolved_with(&allowing, &token(TOKEN_PROJECT), &one("gpt-4o"))
        .await
        .unwrap_err();
    assert_eq!(
        error_type(&project_refused),
        ProxyErrorTypes::ProjectModelAccessDenied.to_string()
    );
    // an empty project allowlist never reaches the project check
    let empty_project = Arc::new(
        Script::new()
            .with_team(team_row(TEAM_ONE))
            .with_project(project_row(r#"{"project_id":"project-1"}"#))
            .with_membership("user-1", "team-1", MEMBER_ALLOWING),
    );
    resolved_with(&empty_project, &token(TOKEN_PROJECT), &one("gpt-4o"))
        .await
        .unwrap();
}

#[tokio::test]
async fn resource_missing_inputs_empty() {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_OWNER)));
    let lookup = team_lookup(&script);
    let team = team_row(TEAM_ONE);
    let missing_token =
        authorized_resources(None, Some(&team), Some(&lookup), ResourceKind::ModelNames).await;
    assert!(missing_token.is_empty());
    let idless = token(TOKEN_PLAIN);
    let missing_ids = authorized_resources(
        Some(&idless),
        Some(&team),
        Some(&lookup),
        ResourceKind::ModelNames,
    )
    .await;
    assert!(missing_ids.is_empty());
    let owned = token(r#"{"token":"hash-1","access_group_ids":["ag-owner"]}"#);
    let missing_db =
        authorized_resources(Some(&owned), Some(&team), None, ResourceKind::ModelNames).await;
    assert!(missing_db.is_empty());
    assert!(script.reads().is_empty());
}

#[tokio::test]
async fn group_team_or_key_owner() {
    let script = Arc::new(
        Script::new()
            .with_group(group_row(GROUP_OWNER))
            .with_group(group_row(GROUP_KEY_OWNED))
            .with_group(group_row(GROUP_OTHER_TEAM)),
    );
    let lookup = team_lookup(&script);
    let team_owned =
        token(r#"{"token":"hash-9","team_id":"team-1","access_group_ids":["ag-owner"]}"#);
    let by_team = authorized_resources(
        Some(&team_owned),
        None,
        Some(&lookup),
        ResourceKind::ModelNames,
    )
    .await;
    assert_eq!(by_team, vec!["claude".to_owned()]);
    let key_owned = token(r#"{"token":"hash-1","access_group_ids":["ag-by-key"]}"#);
    let by_key = authorized_resources(
        Some(&key_owned),
        None,
        Some(&lookup),
        ResourceKind::ModelNames,
    )
    .await;
    assert_eq!(by_key, vec!["claude".to_owned()]);
    let unrelated =
        token(r#"{"token":"hash-9","team_id":"team-3","access_group_ids":["ag-other"]}"#);
    let neither = authorized_resources(
        Some(&unrelated),
        None,
        Some(&lookup),
        ResourceKind::ModelNames,
    )
    .await;
    assert!(neither.is_empty());
}

#[tokio::test]
async fn key_team_precedes_supplied_team() {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_OTHER_TEAM)));
    let lookup = team_lookup(&script);
    let supplied = team_row(TEAM_TWO);
    // the key names its own team, so the supplied team object is not consulted
    let with_team =
        token(r#"{"token":"hash-1","team_id":"team-1","access_group_ids":["ag-other"]}"#);
    let refused = authorized_resources(
        Some(&with_team),
        Some(&supplied),
        Some(&lookup),
        ResourceKind::ModelNames,
    )
    .await;
    assert!(refused.is_empty());
    // a key without a team falls back to the supplied object's id
    let teamless = token(r#"{"token":"hash-1","access_group_ids":["ag-other"]}"#);
    let granted = authorized_resources(
        Some(&teamless),
        Some(&supplied),
        Some(&lookup),
        ResourceKind::ModelNames,
    )
    .await;
    assert_eq!(granted, vec!["claude".to_owned()]);
}

#[tokio::test]
async fn group_error_silent_skip() {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_OWNER)));
    let lookup = team_lookup(&script);
    let valid_token = token(r#"{"token":"hash-1","access_group_ids":["ag-missing","ag-owner"]}"#);
    let resources = authorized_resources(
        Some(&valid_token),
        None,
        Some(&lookup),
        ResourceKind::ModelNames,
    )
    .await;
    assert_eq!(resources, vec!["claude".to_owned()]);
}

#[tokio::test]
async fn unreadable_group_is_unavailable() {
    let script = Arc::new(Script::new());
    let lookup = team_lookup(&script);
    // the load skips a group it cannot read, and asking for that group again reports it unavailable
    let groups = GroupResources::load(Some(&lookup), &["ag-missing".to_owned()]).await;
    let failure = groups.access_group("ag-missing").unwrap_err();
    assert!(matches!(failure, AccessGroupError::Unavailable { .. }));
}

#[tokio::test]
async fn resource_field_matrix_dedup() {
    let script = Arc::new(
        Script::new()
            .with_group(group_row(GROUP_MATRIX_ONE))
            .with_group(group_row(GROUP_MATRIX_TWO)),
    );
    let lookup = team_lookup(&script);
    let valid_token =
        token(r#"{"token":"hash-1","team_id":"team-1","access_group_ids":["ag-one","ag-two"]}"#);
    let models = authorized_resources(
        Some(&valid_token),
        None,
        Some(&lookup),
        ResourceKind::ModelNames,
    )
    .await;
    let mcp = authorized_resources(
        Some(&valid_token),
        None,
        Some(&lookup),
        ResourceKind::McpServerIds,
    )
    .await;
    let agents = authorized_resources(
        Some(&valid_token),
        None,
        Some(&lookup),
        ResourceKind::AgentIds,
    )
    .await;
    assert_eq!(models, vec!["gpt-4o".to_owned(), "claude".to_owned()]);
    assert_eq!(mcp, vec!["mcp-1".to_owned(), "mcp-2".to_owned()]);
    assert_eq!(agents, vec!["agent-1".to_owned(), "agent-2".to_owned()]);
}

#[tokio::test]
async fn key_group_false_on_denial() {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_OWNER)));
    let lookup = team_lookup(&script);
    let setup = Setup::new();
    let context = setup.context();
    let valid_token =
        token(r#"{"token":"hash-1","team_id":"team-1","access_group_ids":["ag-owner"]}"#);
    let refused = key_access_group_grants_model(
        &one("gpt-4o"),
        Some(&valid_token),
        None,
        Some(&lookup),
        &context,
    )
    .await
    .unwrap();
    assert!(!refused);
    let permitted = key_access_group_grants_model(
        &one("claude"),
        Some(&valid_token),
        None,
        Some(&lookup),
        &context,
    )
    .await
    .unwrap();
    assert!(permitted);
    // no group owns the key, so the resources are empty and the answer is false
    let stranger =
        token(r#"{"token":"hash-9","team_id":"team-9","access_group_ids":["ag-owner"]}"#);
    let unauthorized = key_access_group_grants_model(
        &one("claude"),
        Some(&stranger),
        None,
        Some(&lookup),
        &context,
    )
    .await
    .unwrap();
    assert!(!unauthorized);
}

#[tokio::test]
async fn matcher_other_error_propagates() {
    let script = Arc::new(Script::new().with_group(group_row(GROUP_OWNER)));
    let lookup = team_lookup(&script);
    let setup = Setup {
        aliases: BTreeMap::new(),
        max_depth: 0,
    };
    let context = setup.context();
    let valid_token =
        token(r#"{"token":"hash-1","team_id":"team-1","access_group_ids":["ag-owner"]}"#);
    let failure = key_access_group_grants_model(
        &one("claude"),
        Some(&valid_token),
        None,
        Some(&lookup),
        &context,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        failure,
        EntityModelAccessError::RecurseDepth { .. }
    ));
}
