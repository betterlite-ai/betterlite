#![cfg(test)]

mod access_groups;
mod view;

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::proxy_error_types::{ModelAccessObject, ProxyErrorTypes};
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;
use betterlite_models::catalog::BundledCatalog;
use indexmap::IndexMap;

use super::access_groups::{AccessGroupError, AccessGroupLookup, AccessGroupResources};
use super::decision::{
    CompactionContext, ModelAccessContext, can_object_call_model, check_model_access,
};
use super::denial::{
    ModelAccessDecision, ModelAccessDeniedHttpException, model_access_object_name,
    model_denied_client_message,
};
use super::grant::{
    ModelGrantPattern, key_model_aliases, live_team_alias_target, resolve_key_models,
    resolve_team_alias,
};
use super::pattern::{
    CatalogProviderInference, IdentifiedModel, ProviderInference, UnresolvedProviderInference,
    is_model_allowed_by_pattern, is_wildcard_pattern, matches_wildcard_patterns,
};
use super::view::{Deployment, DeploymentView, ModelAccessView, ModelGroupAlias};
use crate::auth_request_models::RequestedModels;

const RECURSE_DEPTH: i64 = 100;

#[derive(Debug)]
struct FixedProvider(BTreeMap<String, IdentifiedModel>);

impl ProviderInference for FixedProvider {
    fn identify(&self, model: &str) -> Option<IdentifiedModel> {
        self.0.get(model).cloned()
    }
}

#[derive(Debug)]
struct FixedGroups(BTreeMap<String, AccessGroupResources>);

impl AccessGroupLookup for FixedGroups {
    fn access_group(
        &self,
        access_group_id: &str,
    ) -> Result<AccessGroupResources, AccessGroupError> {
        self.0
            .get(access_group_id)
            .cloned()
            .ok_or_else(|| AccessGroupError::Unavailable {
                access_group_id: access_group_id.to_owned(),
            })
    }
}

fn context<'context>(
    view: Option<&'context dyn ModelAccessView>,
    aliases: &'context BTreeMap<String, String>,
    provider: &'context dyn ProviderInference,
) -> ModelAccessContext<'context> {
    ModelAccessContext {
        view,
        global_model_aliases: aliases,
        compaction: None,
        provider,
        max_recurse_depth: RECURSE_DEPTH,
    }
}

fn deployment(model_name: &str) -> Deployment {
    Deployment {
        model_name: model_name.to_owned(),
        access_groups: Vec::new(),
        team_id: None,
        team_public_model_name: None,
    }
}

fn grant(models: &[&str]) -> ModelGrantPattern {
    ModelGrantPattern {
        models: models.iter().map(|model| (*model).to_owned()).collect(),
        team_model_aliases: BTreeMap::new(),
        key_model_aliases: BTreeMap::new(),
        team_id: None,
    }
}

fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

fn empty_provider() -> FixedProvider {
    FixedProvider(BTreeMap::new())
}

// the provider inference a wildcard allowlist reads: one bare id, and one prefixed id the inference cannot consume
fn wildcard_provider() -> FixedProvider {
    let mut identities = BTreeMap::new();
    identities.insert(
        "gpt-4o".to_owned(),
        IdentifiedModel {
            provider: "openai".to_owned(),
            model: "gpt-4o".to_owned(),
        },
    );
    identities.insert(
        "bedrockz/anthropic.claude-3".to_owned(),
        IdentifiedModel {
            provider: "bedrock".to_owned(),
            model: "bedrockz/anthropic.claude-3".to_owned(),
        },
    );
    // a namespace the inference consumed and restated as a provider model
    identities.insert(
        "openai-proxy/chat".to_owned(),
        IdentifiedModel {
            provider: "openai".to_owned(),
            model: "gpt-4o".to_owned(),
        },
    );
    FixedProvider(identities)
}

fn empty_aliases() -> BTreeMap<String, String> {
    BTreeMap::new()
}

fn denial_of(decision: ModelAccessDecision) -> ModelAccessDeniedProxyException {
    match decision {
        ModelAccessDecision::Denied(denial) => *denial,
        allowed @ (ModelAccessDecision::Allowed
        | ModelAccessDecision::RecurseDepthExceeded { .. }) => {
            panic!("expected a denial, got {allowed:?}")
        }
    }
}

#[test]
fn literal_model() {
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(None, &aliases, &provider);
    let allowed = grant(&["gpt-4o"]);

    assert!(check_model_access("gpt-4o", &allowed, &ctx));
    assert!(!check_model_access("gpt-4o-mini", &allowed, &ctx));
    assert!(!check_model_access("gpt", &allowed, &ctx));

    let decision = can_object_call_model(
        &one("gpt-4o-mini"),
        &allowed,
        ModelAccessObject::Key,
        &ctx,
        0,
    );
    let denial = denial_of(decision);
    assert_eq!(
        denial.as_proxy_exception().message,
        model_denied_client_message("gpt-4o-mini")
    );
    assert_eq!(denial.as_proxy_exception().param.as_deref(), Some("model"));
    assert_eq!(denial.as_proxy_exception().code, "403");

    // an empty allowlist is unrestricted access, and both wildcard sentinels grant every model
    assert!(check_model_access("any-model", &grant(&[]), &ctx));
    assert!(check_model_access("any-model", &grant(&["*"]), &ctx));
    assert!(check_model_access(
        "any-model",
        &grant(&["all-proxy-models"]),
        &ctx
    ));
    assert!(check_model_access(
        "gpt-4o-mini",
        &grant(&["gpt-4o", "gpt-4o-mini"]),
        &ctx
    ));
}

#[test]
fn provider_wildcard() {
    assert!(is_wildcard_pattern("openai/*"));
    assert!(!is_wildcard_pattern("openai/gpt-4o"));
    assert!(!is_model_allowed_by_pattern("gpt-4o", "openai/gpt-4o"));
    // an exact pattern grants nothing: only one carrying `*` is a wildcard
    assert!(!is_model_allowed_by_pattern("gpt-4o", "gpt-4o"));
    assert!(is_model_allowed_by_pattern("gpt-4o", "gpt-*"));
    // Python's re.match with a trailing `$` also accepts one trailing newline
    assert!(is_model_allowed_by_pattern("gpt-4o\n", "gpt-*"));
    assert!(!is_model_allowed_by_pattern("gpt-4o\nmore", "gpt-*"));
    assert!(!is_model_allowed_by_pattern("bedrock/gpt-4o", "openai/*"));
    // a pattern the engine cannot compile denies instead of failing the request
    assert!(!is_model_allowed_by_pattern("a", "a["));
    assert!(!is_model_allowed_by_pattern("a", "a[b-"));
    assert!(!is_model_allowed_by_pattern("a", "a[*"));
    assert!(!is_model_allowed_by_pattern("a", "a(*"));
}

#[test]
fn provider_wildcard_grants_after_provider_inference() {
    let aliases = empty_aliases();
    let provider = wildcard_provider();
    let ctx = context(None, &aliases, &provider);
    let models = grant(&["openai/*"]);

    // a model that carries the namespace matches directly
    assert!(check_model_access("openai/gpt-4o", &models, &ctx));
    // an unprefixed model is matched after provider inference
    assert!(check_model_access("gpt-4o", &models, &ctx));
    // an unconsumed namespace must not be rebuilt into a provider model
    assert!(!check_model_access(
        "bedrockz/anthropic.claude-3",
        &models,
        &ctx
    ));
    // not even when the pattern names the provider the inference reported for the full string
    assert!(!check_model_access(
        "bedrockz/anthropic.claude-3",
        &grant(&["bedrock/*"]),
        &ctx
    ));
    // a namespace the inference did consume is rebuilt into its provider model
    assert!(check_model_access("openai-proxy/chat", &models, &ctx));
}

#[test]
fn provider_wildcard_keeps_pythons_regex_syntax() {
    // a lookahead the Rust regex crate cannot compile still narrows an allowlist wildcard
    let lookahead = "openai/(?!gpt-4o).*";
    assert!(is_model_allowed_by_pattern("openai/gpt-4.1", lookahead));
    assert!(!is_model_allowed_by_pattern("openai/gpt-4o", lookahead));
    // a backreference the Rust regex crate cannot compile still matches one
    let backreference = "(gpt)-\\1-*";
    assert!(is_model_allowed_by_pattern("gpt-gpt-mini", backreference));
    assert!(!is_model_allowed_by_pattern(
        "gpt-other-mini",
        backreference
    ));
    // Python's \w is Unicode-aware, and the allowlist engine follows it
    assert!(is_model_allowed_by_pattern("\u{e9}llo", "\\w*"));
}

#[test]
fn provider_wildcard_requires_a_known_provider() {
    let aliases = empty_aliases();
    let provider = wildcard_provider();
    let ctx = context(None, &aliases, &provider);

    // a model the inference cannot place is not granted by a provider wildcard
    assert!(!check_model_access(
        "mystery-model",
        &grant(&["openai/*"]),
        &ctx
    ));
    assert!(!matches_wildcard_patterns(
        "mystery-model",
        &["openai/*".to_owned(), "anthropic/*".to_owned()],
        &provider
    ));
    assert!(matches_wildcard_patterns(
        "vertex_ai/gemini-pro",
        &["vertex_ai/*".to_owned()],
        &provider
    ));
    // the provider-inferred branch runs only for a model the bare pattern does not match
    assert!(matches_wildcard_patterns(
        "gpt-4o",
        &["openai/*".to_owned()],
        &provider
    ));
    // the stage's inference for a server without a catalog never places a model
    assert!(
        UnresolvedProviderInference
            .identify("openai/gpt-4o")
            .is_none()
    );
}

#[test]
fn catalog_provider_inference_reads_the_model_catalog() {
    let mut bundled = BundledCatalog::default();
    let catalog = bundled.get();
    let inference = CatalogProviderInference {
        catalog: Some(catalog.as_ref()),
    };
    let identified = inference.identify("gpt-4o").unwrap();
    assert_eq!(identified.provider, "openai");
    assert_eq!(identified.model, "gpt-4o");
    let prefixed = inference.identify("openai/gpt-4o").unwrap();
    assert_eq!(prefixed.provider, "openai");
    assert_eq!(prefixed.model, "gpt-4o");
    assert!(inference.identify("not-a-provider/gpt-4o").is_none());
    assert!(inference.identify("unlisted-model").is_none());
    // an id no exact entry carries falls to the catalog's fallback generalization rules
    let generalized = inference.identify("claude-newfamily-5").unwrap();
    assert_eq!(generalized.provider, "anthropic");
    assert_eq!(generalized.model, "claude-newfamily-5");
    let bare = CatalogProviderInference { catalog: None };
    assert!(bare.identify("gpt-4o").is_none());
}

// the aliases and the router view the liveness rules read
fn live_alias_fixture() -> (DeploymentView, BTreeMap<String, String>) {
    let view = DeploymentView {
        deployments: vec![deployment("gpt-4o")],
        model_group_alias: IndexMap::new(),
    };
    let mut aliases = BTreeMap::new();
    aliases.insert("team-alias".to_owned(), "gpt-4o".to_owned());
    aliases.insert("stale".to_owned(), "model_name_team-a_1".to_owned());
    aliases.insert("other-team".to_owned(), "model_name_team-b_2".to_owned());
    (view, aliases)
}

#[test]
fn team_alias_live_target() {
    let (mut view, aliases) = live_alias_fixture();

    assert_eq!(
        live_team_alias_target("team-alias", &aliases, Some("team-a"), Some(&view)),
        "gpt-4o"
    );
    // a target of the caller's team with no deployment is a deleted deployment, so the alias stays
    assert_eq!(
        live_team_alias_target("stale", &aliases, Some("team-a"), Some(&view)),
        "stale"
    );
    // a target that belongs to no caller team is never treated as deleted
    assert_eq!(
        live_team_alias_target("other-team", &aliases, Some("team-a"), Some(&view)),
        "model_name_team-b_2"
    );
    assert_eq!(
        live_team_alias_target("unaliased", &aliases, Some("team-a"), Some(&view)),
        "unaliased"
    );
    view.deployments.push(deployment("model_name_team-a_1"));
    assert_eq!(
        live_team_alias_target("stale", &aliases, Some("team-a"), Some(&view)),
        "model_name_team-a_1"
    );
    // without a team the target cannot be attributed to a deleted team deployment
    assert_eq!(
        live_team_alias_target("stale", &aliases, None, Some(&view)),
        "model_name_team-a_1"
    );
    // without a router the target is taken as written
    assert_eq!(
        live_team_alias_target("stale", &aliases, Some("team-a"), None),
        "model_name_team-a_1"
    );
}

#[test]
fn team_alias_resolution_rewrites_a_list() {
    let (mut view, aliases) = live_alias_fixture();
    let requested = RequestedModels::Many(vec!["team-alias".to_owned(), "stale".to_owned()]);

    // each entry resolves on its own: the live target rewrites, the deleted deployment's stays
    assert_eq!(
        resolve_team_alias(&requested, &aliases, Some("team-a"), Some(&view)),
        RequestedModels::Many(vec!["gpt-4o".to_owned(), "stale".to_owned()])
    );
    view.deployments.push(deployment("model_name_team-a_1"));
    assert_eq!(
        resolve_team_alias(&requested, &aliases, Some("team-a"), Some(&view)),
        RequestedModels::Many(vec!["gpt-4o".to_owned(), "model_name_team-a_1".to_owned()])
    );
    assert_eq!(
        resolve_team_alias(&one("team-alias"), &BTreeMap::new(), Some("team-a"), None),
        one("team-alias")
    );
    assert_eq!(
        resolve_team_alias(
            &RequestedModels::None,
            &aliases,
            Some("team-a"),
            Some(&view)
        ),
        RequestedModels::None
    );
}

#[test]
fn team_alias_grants_the_alias_name() {
    let (view, aliases) = live_alias_fixture();
    let provider = empty_provider();
    let no_aliases = empty_aliases();
    let mut team_grant = grant(&["gpt-4o"]);
    team_grant.team_model_aliases = aliases;
    team_grant.team_id = Some("team-a".to_owned());
    let ctx = context(Some(&view), &no_aliases, &provider);
    assert!(check_model_access("team-alias", &team_grant, &ctx));
    assert!(check_model_access("stale", &team_grant, &ctx));
    // a team without the alias does not gain it
    team_grant.team_id = Some("team-b".to_owned());
    assert!(!check_model_access("unaliased", &team_grant, &ctx));
}

#[test]
fn key_alias() {
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(None, &aliases, &provider);

    let mut key_alias = grant(&["gpt-4o"]);
    key_alias
        .key_model_aliases
        .insert("fast".to_owned(), "gpt-4o".to_owned());
    // the alias dispatches to its target, which the allowlist grants
    assert!(
        can_object_call_model(&one("fast"), &key_alias, ModelAccessObject::Key, &ctx, 0)
            .is_allowed()
    );
    // the alias name itself grants nothing; only the dispatched target is checked
    assert!(!check_model_access("fast", &key_alias, &ctx));

    // a dispatched alias target is the only model checked
    let mut outside = grant(&["gpt-4o"]);
    outside
        .key_model_aliases
        .insert("fast".to_owned(), "gpt-4o-mini".to_owned());
    assert!(
        !can_object_call_model(&one("fast"), &outside, ModelAccessObject::Key, &ctx, 0)
            .is_allowed()
    );

    // the alias map rewrites the team alias first and the dispatched name again after the global map
    let mut team_aliases = BTreeMap::new();
    team_aliases.insert("team-fast".to_owned(), "fast".to_owned());
    let mut chained = grant(&["gpt-4o"]);
    chained.team_model_aliases = team_aliases;
    chained
        .key_model_aliases
        .insert("fast".to_owned(), "gpt-4o".to_owned());
    assert!(
        can_object_call_model(&one("team-fast"), &chained, ModelAccessObject::Key, &ctx, 0)
            .is_allowed()
    );
}

#[test]
fn key_alias_map_keeps_only_string_targets() {
    let raw: BTreeMap<String, JsonData> = BTreeMap::from([
        ("fast".to_owned(), JsonData::String("gpt-4o".to_owned())),
        ("broken".to_owned(), JsonData::Bool(true)),
        ("nested".to_owned(), JsonData::Array(Vec::new())),
    ]);
    assert_eq!(
        key_model_aliases(&raw),
        BTreeMap::from([("fast".to_owned(), "gpt-4o".to_owned())])
    );
    let broken_only: BTreeMap<String, JsonData> =
        BTreeMap::from([("broken".to_owned(), JsonData::Null)]);
    assert!(key_model_aliases(&broken_only).is_empty());
    let valid: BTreeMap<String, JsonData> =
        BTreeMap::from([("fast".to_owned(), JsonData::String("gpt-4o".to_owned()))]);
    assert_eq!(
        key_model_aliases(&valid),
        BTreeMap::from([("fast".to_owned(), "gpt-4o".to_owned())])
    );
    assert!(key_model_aliases(&BTreeMap::new()).is_empty());
}

// a proxy whose model list holds one shared deployment and one other team's deployment
fn sentinel_view() -> DeploymentView {
    let mut view = DeploymentView::default();
    view.deployments.push(deployment("gpt-4o"));
    let mut other_team = deployment("team-only");
    other_team.team_id = Some("team-b".to_owned());
    view.deployments.push(other_team);
    view
}

#[test]
fn all_team_models() {
    let view = sentinel_view();
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(Some(&view), &aliases, &provider);

    let mut team_grant = grant(&["all-team-models"]);
    team_grant.team_id = Some("team-a".to_owned());
    assert!(check_model_access("gpt-4o", &team_grant, &ctx));
    // the sentinel expands to the proxy's own model names, never to another team's deployment
    assert!(!check_model_access("team-only", &team_grant, &ctx));
    assert!(!check_model_access("all-team-models", &team_grant, &ctx));

    // without a team the sentinel is a scoped name and grants nothing else
    let unscoped = grant(&["all-team-models"]);
    assert!(!check_model_access("gpt-4o", &unscoped, &ctx));
    assert!(check_model_access("all-team-models", &unscoped, &ctx));

    // without a router view the sentinel stays in the list
    let no_view = context(None, &aliases, &provider);
    assert!(!check_model_access("gpt-4o", &team_grant, &no_view));
}

#[test]
fn all_team_models_expands_into_the_proxy_model_list() {
    let view = sentinel_view();
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(Some(&view), &aliases, &provider);
    let mut team_grant = grant(&["all-team-models"]);
    team_grant.team_id = Some("team-a".to_owned());

    // mixed lists keep their own entries and gain the proxy model names once
    let mixed = grant(&["gpt-4o", "all-team-models", "gpt-4o"]);
    assert!(check_model_access("gpt-4o", &mixed, &ctx));
    assert!(!check_model_access("team-only", &mixed, &ctx));
    let view_ref: &dyn ModelAccessView = &view;
    assert_eq!(
        super::sentinel::resolve_team_model_sentinel(&mixed.models, Some(view_ref), Some("team-a")),
        vec!["gpt-4o".to_owned()]
    );
}

#[test]
fn all_team_models_without_a_proxy_model_list() {
    let aliases = empty_aliases();
    let provider = empty_provider();
    let mut team_grant = grant(&["all-team-models"]);
    team_grant.team_id = Some("team-a".to_owned());
    let mixed = grant(&["gpt-4o", "all-team-models", "gpt-4o"]);
    // a mixed list keeps a duplicated entry only when the proxy itself lists no model
    let empty_view = DeploymentView::default();
    let empty_ref: &dyn ModelAccessView = &empty_view;
    assert_eq!(
        super::sentinel::resolve_team_model_sentinel(
            &mixed.models,
            Some(empty_ref),
            Some("team-a")
        ),
        vec!["gpt-4o".to_owned(), "gpt-4o".to_owned()]
    );

    let empty_ctx = context(Some(&empty_view), &aliases, &provider);
    assert!(!check_model_access("gpt-4o", &team_grant, &empty_ctx));
    assert!(check_model_access(
        "all-team-models",
        &team_grant,
        &empty_ctx
    ));
}

#[test]
fn all_team_models_key_sentinel() {
    // the key sentinel expands to the parent team's allowlist, or to unrestricted without a team
    assert_eq!(
        resolve_key_models(
            &["all-team-models".to_owned()],
            &["gpt-4o".to_owned()],
            Some("team-a")
        ),
        vec!["gpt-4o".to_owned()]
    );
    assert!(
        resolve_key_models(
            &["all-team-models".to_owned()],
            &["gpt-4o".to_owned()],
            None
        )
        .is_empty()
    );
    assert_eq!(
        resolve_key_models(&["gpt-4o".to_owned()], &["gpt-4o-mini".to_owned()], None),
        vec!["gpt-4o".to_owned()]
    );
}

#[test]
fn generic_model_denial() {
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(None, &aliases, &provider);
    let secret_allowlist = grant(&["gpt-4o", "internal-model"]);

    let decision = can_object_call_model(
        &one("private-model"),
        &secret_allowlist,
        ModelAccessObject::Key,
        &ctx,
        0,
    );
    let denial = denial_of(decision);
    let exception = denial.as_proxy_exception();
    assert_eq!(
        exception.message,
        "The requested model 'private-model' is not available for this API key, or the model name is invalid. Check the models available to you and try again."
    );
    assert_eq!(
        exception.message,
        model_denied_client_message("private-model")
    );
    assert_eq!(exception.error_type, "key_model_access_denied");
    assert_eq!(exception.param.as_deref(), Some("model"));
    assert_eq!(exception.code, "403");

    // the typed cause keeps the allowlist for diagnostics; the client message never carries it
    let internal = denial.sanitized_internal_message();
    assert!(internal.contains("key not allowed to access model"));
    assert!(internal.contains("internal-model"));
    assert!(internal.contains("Tried to access private-model"));
    assert!(!exception.message.contains("internal-model"));
    assert!(!exception.message.contains("gpt-4o"));
}

#[test]
fn model_access_denied_error_types_name_each_object() {
    // every object maps to its own error type, and the internal message names it
    assert_eq!(model_access_object_name(ModelAccessObject::Key), "key");
    assert_eq!(model_access_object_name(ModelAccessObject::Team), "team");
    assert_eq!(model_access_object_name(ModelAccessObject::User), "user");
    assert_eq!(model_access_object_name(ModelAccessObject::Org), "org");
    assert_eq!(
        model_access_object_name(ModelAccessObject::Project),
        "project"
    );
    assert_eq!(model_access_object_name(ModelAccessObject::Agent), "agent");
    assert_eq!(
        ProxyErrorTypes::model_access_denied_for(ModelAccessObject::Team).to_string(),
        "team_model_access_denied"
    );
}

#[test]
fn model_access_denied_covers_lists() {
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(None, &aliases, &provider);
    let secret_allowlist = grant(&["gpt-4o", "internal-model"]);

    // a list request reports the first denied model under the caller's object type
    let list = can_object_call_model(
        &RequestedModels::Many(vec!["gpt-4o".to_owned(), "private-model".to_owned()]),
        &secret_allowlist,
        ModelAccessObject::Team,
        &ctx,
        0,
    );
    let list_denial = denial_of(list);
    assert_eq!(
        list_denial.as_proxy_exception().error_type,
        "team_model_access_denied"
    );
    assert!(
        list_denial
            .as_proxy_exception()
            .message
            .contains("private-model")
    );
    // a list the allowlist covers fully is allowed
    assert!(
        can_object_call_model(
            &RequestedModels::Many(vec!["gpt-4o".to_owned(), "internal-model".to_owned()]),
            &secret_allowlist,
            ModelAccessObject::Team,
            &ctx,
            0,
        )
        .is_allowed()
    );
}

#[test]
fn model_access_denied_http_exception_becomes_auth_error() {
    // a denial raised as a FastAPI HTTPException keeps its status and becomes auth_error
    let raised = ModelAccessDeniedHttpException::new(
        "Role=internal_user not allowed to call model=gpt-4o".to_owned(),
        403,
        model_denied_client_message("gpt-4o"),
    );
    let translated = raised.as_proxy_exception();
    assert_eq!(raised.status_code(), 403);
    assert_eq!(translated.as_proxy_exception().error_type, "auth_error");
    assert_eq!(
        translated.as_proxy_exception().param.as_deref(),
        Some("None")
    );
    assert_eq!(translated.as_proxy_exception().code, "403");
    assert_eq!(
        translated.as_proxy_exception().message,
        model_denied_client_message("gpt-4o")
    );
    assert!(
        translated
            .sanitized_internal_message()
            .contains("Role=internal_user")
    );
}

fn compaction_context<'context>(
    provider: &'context FixedProvider,
    aliases: &'context BTreeMap<String, String>,
    parent_model: &'context str,
    compactor: &'context str,
) -> ModelAccessContext<'context> {
    ModelAccessContext {
        view: None,
        global_model_aliases: aliases,
        compaction: Some(CompactionContext {
            parent_model,
            compactor,
        }),
        provider,
        max_recurse_depth: RECURSE_DEPTH,
    }
}

#[test]
fn alias_chain_and_compaction_parent_are_checked() {
    let provider = empty_provider();
    let no_aliases = empty_aliases();
    let allowed = grant(&["gpt-4o"]);
    // the global alias target is checked next to the requested name
    let mut aliases = BTreeMap::new();
    aliases.insert("public".to_owned(), "gpt-4o".to_owned());
    let ctx = context(None, &aliases, &provider);
    assert!(
        can_object_call_model(&one("public"), &allowed, ModelAccessObject::Key, &ctx, 0)
            .is_allowed()
    );
    // without the alias the same name is denied
    let plain_ctx = context(None, &no_aliases, &provider);
    assert!(
        !can_object_call_model(
            &one("public"),
            &allowed,
            ModelAccessObject::Key,
            &plain_ctx,
            0
        )
        .is_allowed()
    );

    // the compactor may call the parent model it summarizes
    let compacted = compaction_context(&provider, &no_aliases, "gpt-4o", "gpt-4o-compact");
    assert!(
        can_object_call_model(
            &one("gpt-4o-compact"),
            &allowed,
            ModelAccessObject::Key,
            &compacted,
            0
        )
        .is_allowed()
    );
}

#[test]
fn compaction_parent_is_only_reached_by_its_compactor() {
    let provider = empty_provider();
    let no_aliases = empty_aliases();
    let allowed = grant(&["gpt-4o"]);
    let compacted = compaction_context(&provider, &no_aliases, "gpt-4o", "gpt-4o-compact");

    assert!(
        !can_object_call_model(
            &one("gpt-4o-mini"),
            &allowed,
            ModelAccessObject::Key,
            &compacted,
            0
        )
        .is_allowed()
    );
    let compaction = compacted.compaction.unwrap();
    assert_eq!(compaction.parent_for("gpt-4o-compact"), Some("gpt-4o"));
    assert_eq!(compaction.parent_for("gpt-4o"), None);
}

#[test]
fn empty_alias_target_is_not_a_potential_model() {
    let provider = empty_provider();
    let mut aliases = BTreeMap::new();
    aliases.insert("empty-target".to_owned(), String::new());
    let ctx = context(None, &aliases, &provider);
    assert!(
        !can_object_call_model(
            &one("empty-target"),
            &grant(&["gpt-4o"]),
            ModelAccessObject::Key,
            &ctx,
            0
        )
        .is_allowed()
    );

    // the global map decides by presence, so its empty entry keeps the router alias of the same name out
    let mut view = DeploymentView::default();
    view.model_group_alias.insert(
        "empty-target".to_owned(),
        ModelGroupAlias {
            model: "gpt-4o".to_owned(),
            hidden: false,
        },
    );
    let router_ctx = context(Some(&view), &aliases, &provider);
    assert!(
        !can_object_call_model(
            &one("empty-target"),
            &grant(&["gpt-4o"]),
            ModelAccessObject::Key,
            &router_ctx,
            0
        )
        .is_allowed()
    );
}

#[test]
fn key_alias_after_the_global_alias_checks_only_the_final_target() {
    let provider = empty_provider();
    let mut aliases = BTreeMap::new();
    aliases.insert("alias".to_owned(), "target".to_owned());
    let ctx = context(None, &aliases, &provider);
    let mut aliased_grant = grant(&["target"]);
    aliased_grant
        .key_model_aliases
        .insert("target".to_owned(), "elsewhere".to_owned());

    // the key alias rewrites the global alias target, and only the final target is checked
    assert!(
        !can_object_call_model(
            &one("alias"),
            &aliased_grant,
            ModelAccessObject::Key,
            &ctx,
            0
        )
        .is_allowed()
    );
}

#[test]
fn team_alias_chains_into_the_global_alias() {
    let provider = empty_provider();
    let mut aliases = BTreeMap::new();
    aliases.insert("public".to_owned(), "gpt-4o".to_owned());
    let ctx = context(None, &aliases, &provider);

    // a team alias resolves to the global alias target that is then checked
    let mut chained = grant(&["gpt-4o"]);
    chained
        .team_model_aliases
        .insert("team-public".to_owned(), "public".to_owned());
    assert!(
        can_object_call_model(&one("public"), &chained, ModelAccessObject::Key, &ctx, 0)
            .is_allowed()
    );
    assert!(
        can_object_call_model(
            &one("team-public"),
            &chained,
            ModelAccessObject::Key,
            &ctx,
            0
        )
        .is_allowed()
    );
}

#[test]
fn router_alias_is_the_fallback_for_the_global_map() {
    let provider = empty_provider();
    let no_aliases = empty_aliases();
    let allowed = grant(&["gpt-4o"]);
    let mut view = DeploymentView::default();
    view.model_group_alias.insert(
        "router-alias".to_owned(),
        ModelGroupAlias {
            model: "gpt-4o".to_owned(),
            hidden: false,
        },
    );
    let router_ctx = context(Some(&view), &no_aliases, &provider);
    assert!(
        can_object_call_model(
            &one("router-alias"),
            &allowed,
            ModelAccessObject::Key,
            &router_ctx,
            0
        )
        .is_allowed()
    );
    // the global map does not know the name, and without the router view nothing grants it
    let plain_ctx = context(None, &no_aliases, &provider);
    assert!(
        !can_object_call_model(
            &one("router-alias"),
            &allowed,
            ModelAccessObject::Key,
            &plain_ctx,
            0
        )
        .is_allowed()
    );
}

#[test]
fn recurse_depth_is_bounded_by_the_configured_limit() {
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(None, &aliases, &provider);
    let allowed = grant(&["gpt-4o"]);

    assert!(
        can_object_call_model(
            &one("gpt-4o"),
            &allowed,
            ModelAccessObject::Key,
            &ctx,
            RECURSE_DEPTH - 1
        )
        .is_allowed()
    );
    assert_eq!(
        can_object_call_model(
            &one("gpt-4o"),
            &allowed,
            ModelAccessObject::Key,
            &ctx,
            RECURSE_DEPTH
        ),
        ModelAccessDecision::RecurseDepthExceeded {
            model: "gpt-4o".to_owned()
        }
    );
}

#[test]
fn recurse_depth_counts_each_list_entry() {
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(None, &aliases, &provider);
    let allowed = grant(&["gpt-4o"]);

    assert_eq!(
        can_object_call_model(
            &RequestedModels::Many(vec!["gpt-4o".to_owned()]),
            &allowed,
            ModelAccessObject::Key,
            &ctx,
            RECURSE_DEPTH - 1
        ),
        ModelAccessDecision::RecurseDepthExceeded {
            model: "gpt-4o".to_owned()
        }
    );
    // a list request recurses one level per entry, so a depth limit of 1 rejects every entry
    let shallow = ModelAccessContext {
        view: None,
        global_model_aliases: &aliases,
        compaction: None,
        provider: &provider,
        max_recurse_depth: 1,
    };
    assert_eq!(
        can_object_call_model(
            &RequestedModels::Many(vec!["gpt-4o".to_owned()]),
            &allowed,
            ModelAccessObject::Key,
            &shallow,
            0
        ),
        ModelAccessDecision::RecurseDepthExceeded {
            model: "gpt-4o".to_owned()
        }
    );
}

#[test]
fn absent_model_skips_the_depth_guard() {
    let aliases = empty_aliases();
    let provider = empty_provider();
    let ctx = context(None, &aliases, &provider);
    // no requested model means nothing to check
    assert!(
        can_object_call_model(
            &RequestedModels::None,
            &grant(&["gpt-4o"]),
            ModelAccessObject::Key,
            &ctx,
            0
        )
        .is_allowed()
    );
}
