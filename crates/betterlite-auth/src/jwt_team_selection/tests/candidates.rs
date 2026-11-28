use std::collections::BTreeMap;
use std::sync::Arc;

use rstest::rstest;

use super::super::candidates::{
    CandidateContext, CandidateRequest, find_team_with_model_access, team_passthrough_access,
};
use super::support::{
    Script, context, ids, lookup, open_registry, protected_registry, rendered, selection, team,
};
use crate::jwt_team_selection::settings::SelectionSettings;

const TEAM_A: &str = r#"{"team_id": "team-a", "models": ["model-a"]}"#;
const TEAM_B: &str = r#"{"team_id": "team-b", "models": ["model-b"]}"#;
const ALIAS_TEAM: &str = r#"{"team_id": "team-alias", "models": ["real-model"], "litellm_model_table": {"model_aliases": {"friendly": "real-model"}, "created_by": "oracle", "updated_by": "oracle"}}"#;
const NULL_MODELS: &str = r#"{"team_id": "team-a-null", "models": null}"#;
const PROTECTED: &str = "/v1/registered";
const ALLOWED_ROUTE: &str = "/chat/completions";
const DENIED_ROUTE: &str = "/v1/unlisted";
// an auth-enforced pass-through registered on a route the openai_routes group already allows
const OPENAI_ROUTE: &str = "/v1/chat/completions";

fn allowed_route_settings() -> SelectionSettings {
    selection(|fields| fields.routes.team_allowed_routes = vec![ALLOWED_ROUTE.to_owned()])
}

// the route group admits the route, while no entry names it as a path the team grant could match
fn group_route_settings() -> SelectionSettings {
    selection(|fields| fields.routes.team_allowed_routes = vec!["openai_routes".to_owned()])
}

fn request<'request>(
    team_ids: &'request std::collections::BTreeSet<String>,
    model: Option<&'request str>,
    route: &'request str,
) -> CandidateRequest<'request> {
    CandidateRequest {
        team_ids,
        requested_model: model,
        route,
        request_method: Some("POST"),
    }
}

#[rstest]
#[case::no_groups_enforced(true, false, true)]
#[case::no_groups_with_fallback(true, true, false)]
#[case::no_groups_not_enforced(false, false, false)]
#[tokio::test]
async fn no_groups_enforced(#[case] enforced: bool, #[case] fallback: bool, #[case] denied: bool) {
    let script = Arc::new(Script::new());
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let settings = selection(|fields| {
        fields.scopes.team_model_access = enforced;
        fields.routing.fallback_to_db_teams = fallback;
    });
    let team_ids = ids(&[]);
    let call = request(&team_ids, Some("model-a"), ALLOWED_ROUTE);
    let answer = find_team_with_model_access(&call, &deps, &settings).await;
    if denied {
        let (status, _, message) = rendered(answer.unwrap_err());
        assert_eq!(status, 403);
        assert_eq!(
            message,
            "No teams found in token. `enforce_team_based_model_access` is set to True. Token must belong to a team."
        );
    } else {
        assert_eq!(answer.unwrap(), None);
    }
}

#[tokio::test]
async fn candidate_lookup_swallowed() {
    let script = Arc::new(Script::new().with_raw("team-a", r#"{"team_id": 5}"#));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let settings = selection(|fields| fields.routing.team_claim_fallback = true);
    let team_ids = ids(&["team-a", "team-missing"]);
    let call = request(&team_ids, Some("model-a"), ALLOWED_ROUTE);
    assert_eq!(
        find_team_with_model_access(&call, &deps, &settings)
            .await
            .unwrap(),
        None
    );
    assert_eq!(script.events(), vec!["team team-a", "team team-missing"]);
}

#[tokio::test]
async fn candidate_models_null() {
    let script = Arc::new(
        Script::new()
            .with_raw("team-a-null", NULL_MODELS)
            .with_team(TEAM_B),
    );
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let team_ids = ids(&["team-a-null", "team-b"]);
    let call = request(&team_ids, Some("model-b"), ALLOWED_ROUTE);
    let (team_id, row) = find_team_with_model_access(&call, &deps, &allowed_route_settings())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(team_id, "team-b");
    assert_eq!(row.fields.base.team_id, "team-b");
}

// the deviation: a NULL models row does not decode, so the scan treats it as unresolved and the claim
// fallback applies, where LiteLLM reads models=None as a resolved team that is skipped and denies
#[tokio::test]
async fn candidate_models_null_defers_under_fallback() {
    let script = Arc::new(Script::new().with_raw("team-a-null", NULL_MODELS));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let settings = selection(|fields| fields.routing.team_claim_fallback = true);
    let team_ids = ids(&["team-a-null"]);
    let call = request(&team_ids, Some("model-b"), ALLOWED_ROUTE);
    assert_eq!(
        find_team_with_model_access(&call, &deps, &settings)
            .await
            .unwrap(),
        None
    );
    assert_eq!(script.events(), vec!["team team-a-null"]);
}

#[tokio::test]
async fn candidate_no_model() {
    let script = Arc::new(Script::new().with_team(TEAM_A));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let team_ids = ids(&["team-a"]);
    let call = request(&team_ids, None, ALLOWED_ROUTE);
    let (team_id, _row) = find_team_with_model_access(&call, &deps, &allowed_route_settings())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(team_id, "team-a");
    let denied = request(&team_ids, None, DENIED_ROUTE);
    assert_eq!(
        find_team_with_model_access(&denied, &deps, &allowed_route_settings())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn candidate_model_alias() {
    let script = Arc::new(Script::new().with_team(ALIAS_TEAM));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let team_ids = ids(&["team-alias"]);
    let call = request(&team_ids, Some("friendly"), ALLOWED_ROUTE);
    let (team_id, _row) = find_team_with_model_access(&call, &deps, &allowed_route_settings())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(team_id, "team-alias");
}

#[tokio::test]
async fn candidate_model_grant() {
    let script = Arc::new(Script::new().with_team(TEAM_A));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let team_ids = ids(&["team-a"]);
    let granted = request(&team_ids, Some("model-a"), ALLOWED_ROUTE);
    assert_eq!(
        find_team_with_model_access(&granted, &deps, &allowed_route_settings())
            .await
            .unwrap()
            .map(|(team_id, _row)| team_id),
        Some("team-a".to_owned())
    );
    let denied = request(&team_ids, Some("model-b"), ALLOWED_ROUTE);
    let (status, _, message) = rendered(
        find_team_with_model_access(&denied, &deps, &allowed_route_settings())
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 403);
    assert_eq!(
        message,
        "No team has access to the requested model: model-b. Checked teams={'team-a'}. Check `/models` to see all available models."
    );
}

#[tokio::test]
async fn multiple_team_model_selection() {
    let script = Arc::new(Script::new().with_team(TEAM_A).with_team(TEAM_B));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let team_ids = ids(&["team-a", "team-b"]);
    let call = request(&team_ids, Some("model-b"), ALLOWED_ROUTE);
    let (team_id, _row) = find_team_with_model_access(&call, &deps, &allowed_route_settings())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(team_id, "team-b");
}

#[rstest]
#[case::all_unresolved_fallback(true, true)]
#[case::all_unresolved_strict(false, false)]
#[tokio::test]
async fn all_unresolved_fallback(#[case] fallback: bool, #[case] allowed: bool) {
    let script = Arc::new(Script::new());
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let settings = selection(|fields| fields.routing.team_claim_fallback = fallback);
    let team_ids = ids(&["ghost"]);
    let call = request(&team_ids, Some("model-a"), ALLOWED_ROUTE);
    let answer = find_team_with_model_access(&call, &deps, &settings).await;
    if allowed {
        assert_eq!(answer.unwrap(), None);
    } else {
        assert_eq!(rendered(answer.unwrap_err()).0, 403);
    }
}

#[tokio::test]
async fn resolved_model_denied() {
    let script = Arc::new(Script::new().with_team(TEAM_A));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &open_registry(),
        models: &models,
    };
    let settings = selection(|fields| {
        fields.routing.team_claim_fallback = true;
        fields.routes.team_allowed_routes = vec![ALLOWED_ROUTE.to_owned()];
    });
    let team_ids = ids(&["team-a"]);
    let call = request(&team_ids, Some("model-b"), ALLOWED_ROUTE);
    assert_eq!(
        rendered(
            find_team_with_model_access(&call, &deps, &settings)
                .await
                .unwrap_err()
        )
        .0,
        403
    );
}

#[test]
fn pass_through_method_upper() {
    let registry = protected_registry(PROTECTED, "POST");
    assert!(!team_passthrough_access(
        None,
        PROTECTED,
        Some("post"),
        &[],
        &registry
    ));
    assert!(!team_passthrough_access(
        None,
        PROTECTED,
        None,
        &[],
        &registry
    ));
    assert!(team_passthrough_access(
        None,
        "/elsewhere",
        Some("post"),
        &[],
        &registry
    ));
}

#[test]
fn pass_through_config_grant() {
    let registry = protected_registry(PROTECTED, "POST");
    assert!(team_passthrough_access(
        None,
        PROTECTED,
        Some("POST"),
        &[PROTECTED.to_owned()],
        &registry
    ));
}

#[test]
fn pass_through_metadata_grant() {
    let registry = protected_registry(PROTECTED, "POST");
    let row = team(
        r#"{"team_id": "team-a", "models": [], "metadata": {"allowed_passthrough_routes": ["/v1/registered"]}}"#,
    );
    assert!(team_passthrough_access(
        Some(&row),
        PROTECTED,
        Some("POST"),
        &[],
        &registry
    ));
}

#[tokio::test]
async fn pass_through_denial_priority() {
    let candidates = ["team-a", "team-b"];
    let model = "model-a";
    let script = Arc::new(Script::new().with_team(TEAM_A).with_team(TEAM_B));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &protected_registry(OPENAI_ROUTE, "POST"),
        models: &models,
    };
    let team_ids = ids(&candidates);
    let call = request(&team_ids, Some(model), OPENAI_ROUTE);
    let (status, _, message) = rendered(
        find_team_with_model_access(&call, &deps, &group_route_settings())
            .await
            .unwrap_err(),
    );
    assert_eq!(status, 403);
    assert_eq!(
        message,
        "Team not allowed to access passthrough route /v1/chat/completions. Configure `allowed_passthrough_routes` on the team."
    );
}

#[tokio::test]
async fn pass_through_team_denial() {
    let script = Arc::new(Script::new().with_team(TEAM_A));
    let lookup = lookup(&script, true);
    let aliases = BTreeMap::new();
    let models = context(&aliases);
    let deps = CandidateContext {
        lookup: &lookup,
        registry: &protected_registry(OPENAI_ROUTE, "POST"),
        models: &models,
    };
    let team_ids = ids(&["team-a"]);
    let call = request(&team_ids, Some("model-a"), OPENAI_ROUTE);
    assert_eq!(
        rendered(
            find_team_with_model_access(&call, &deps, &group_route_settings())
                .await
                .unwrap_err()
        )
        .0,
        403
    );
}
