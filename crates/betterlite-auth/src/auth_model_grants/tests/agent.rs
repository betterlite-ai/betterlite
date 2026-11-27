use std::sync::Arc;

use rstest::rstest;

use crate::auth_model_grants::access_source::AgentAccessSource;
use crate::auth_model_grants::caller::{CallerChecks, check_agent_caller};
use crate::auth_model_grants::ceiling::check_agent_ceiling;
use crate::auth_model_grants::error::AgentModelGrantsError;
use crate::auth_model_patterns::denial::model_denied_client_message;
use crate::auth_request_models::RequestedModels;
use crate::entity_model_access::error::EntityModelAccessError;

use super::support::{
    AGENT_ALIAS_KEY, AGENT_KEY, AGENT_MODEL_ALIAS_KEY, AGENT_TEAM_KEY, ALLOWING_MEMBER,
    ALLOWING_TEAM, CALLER_KEY, CaseMemberships, DENYING_MEMBER, DENYING_TEAM, Failing, GROUP_TEAM,
    GroupDirectory, POLICY, Script, admission, agent_token, ceiling, context, denial_of, one,
    shallow_context, team_lookup, token,
};

#[rstest]
#[case::falsey_none(RequestedModels::None, Some(AGENT_KEY), true)]
#[case::empty_model(RequestedModels::One(String::new()), Some(AGENT_KEY), true)]
#[case::empty_many(RequestedModels::Many(Vec::new()), Some(AGENT_KEY), true)]
#[case::no_token(RequestedModels::One("gpt-4o".to_owned()), None, true)]
#[case::plain_key(
    RequestedModels::One("gpt-4o".to_owned()),
    Some(r#"{"token":"sk-plain"}"#),
    true
)]
#[case::empty_agent_id(
    RequestedModels::One("gpt-4o".to_owned()),
    Some(r#"{"token":"sk","agent_id":""}"#),
    true
)]
#[case::falsey_without_source(RequestedModels::None, Some(AGENT_KEY), false)]
#[tokio::test]
async fn missing_model_token_agent_skips(
    #[case] model: RequestedModels,
    #[case] fields: Option<&'static str>,
    #[case] with_source: bool,
) {
    let script = Script::plain();
    let key = fields.map(token);
    let as_source: &dyn AgentAccessSource = &script;
    let source = with_source.then_some(as_source);
    check_agent_ceiling(&model, key.as_ref(), source, &context())
        .await
        .unwrap();
    assert!(script.events().is_empty());
}

#[tokio::test]
async fn missing_source_fails_agent_requests() {
    let context = context();
    let agent = token(AGENT_KEY);
    assert!(matches!(
        check_agent_ceiling(&one("gpt-4o"), Some(&agent), None, &context).await,
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
        ..Script::plain()
    };
    check_agent_ceiling(
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
    let script = Script::plain();
    check_agent_ceiling(
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
        ..Script::plain()
    };
    let error = check_agent_ceiling(
        &one("secret-model"),
        Some(&token(AGENT_KEY)),
        Some(&script),
        &context(),
    )
    .await
    .unwrap_err();
    let internal = denial_of(error, "agent_model_access_denied");
    assert_eq!(internal, "agent agent-1 access groups grant no models");
    let AgentModelGrantsError::Denied(denial) = check_agent_ceiling(
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
        ..Script::plain()
    };
    // the first ceiling allows, so the denial can only come from the second one
    let error = check_agent_ceiling(
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
        ..Script::plain()
    };
    check_agent_ceiling(
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
        ..Script::plain()
    };
    // the alias resolves to gpt-4o before the ceiling sees the name
    check_agent_ceiling(
        &one("alias"),
        Some(&token(AGENT_ALIAS_KEY)),
        Some(&script),
        &context(),
    )
    .await
    .unwrap();
    // without the alias map the same request denies, so the resolution is what allowed it
    assert!(matches!(
        check_agent_ceiling(
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
        ..Script::plain()
    };
    let error = check_agent_ceiling(
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
        ..Script::plain()
    };
    // the ceiling holds only the target; the key alias rewrites the request onto it
    check_agent_ceiling(
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
        fails: vec![Failing::Managed],
        ..Script::plain()
    };
    assert!(matches!(
        check_agent_ceiling(
            &one("gpt-4o"),
            Some(&token(AGENT_KEY)),
            Some(&managed),
            &context(),
        )
        .await,
        Err(AgentModelGrantsError::Ceiling(_))
    ));
    let unmanaged = Script {
        fails: vec![Failing::Unmanaged],
        ..Script::plain()
    };
    assert!(matches!(
        check_agent_ceiling(
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
        ..Script::plain()
    };
    assert!(matches!(
        check_agent_ceiling(
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
        ..Script::plain()
    };
    let memberships = CaseMemberships::default();
    let context = context();
    let admission = admission(&memberships, &context);
    check_agent_caller(
        &one("gpt-4o"),
        Some(&token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert!(script.events().is_empty());
    assert!(memberships.asked().is_empty());
}

#[rstest]
#[case::falsey_none(RequestedModels::None, true)]
#[case::empty_model(RequestedModels::One(String::new()), true)]
#[case::empty_many(RequestedModels::Many(Vec::new()), true)]
#[case::no_token(RequestedModels::One("gpt-4o".to_owned()), false)]
#[tokio::test]
async fn caller_falsey_model_and_token_skip(
    #[case] model: RequestedModels,
    #[case] with_token: bool,
) {
    let context = context();
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    // the token carries a caller identity, so only a falsey model or a missing token can skip
    let denying = Script {
        caller: Some(CALLER_KEY.to_owned()),
        user: Some(r#"{"user_id":"caller-1","models":["other-model"]}"#.to_owned()),
        ..Script::plain()
    };
    let key = with_token.then_some(agent_token(AGENT_KEY));
    check_agent_caller(
        &model,
        key.as_ref(),
        Some(&denying),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert!(denying.events().is_empty());
    assert!(memberships.asked().is_empty());
}

#[tokio::test]
async fn unresolved_caller_allows() {
    // a caller identity the source cannot resolve allows without asking for a team or a user
    let context = context();
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    let script = Script::plain();
    check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert_eq!(script.events(), ["caller_auth"]);
    assert!(memberships.asked().is_empty());
}

#[tokio::test]
async fn caller_key_aliases_forwarded() {
    let context = context();
    let memberships = CaseMemberships::allowed(ALLOWING_MEMBER);
    let caller_admission = admission(&memberships, &context);
    // the team and the member allowlist both name gpt-4o, so only the agent key's alias allows "fast"
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(ALLOWING_TEAM.to_owned()),
        ..Script::plain()
    };
    check_agent_caller(
        &one("fast"),
        Some(&agent_token(AGENT_MODEL_ALIAS_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &caller_admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert_eq!(script.events(), ["caller_auth", "caller_team"]);
}

#[tokio::test]
async fn caller_user_reads_key_aliases() {
    let context = context();
    let memberships = CaseMemberships::default();
    let caller_admission = admission(&memberships, &context);
    // without a caller team the user's own models decide through the same agent key alias map
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        user: Some(r#"{"user_id":"caller-1","models":["gpt-4o"]}"#.to_owned()),
        ..Script::plain()
    };
    check_agent_caller(
        &one("fast"),
        Some(&agent_token(AGENT_MODEL_ALIAS_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &caller_admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        script.events(),
        ["caller_auth", "caller_team", "caller_user"]
    );
}

#[tokio::test]
async fn caller_team_before_membership() {
    let context = context();
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(ALLOWING_TEAM.to_owned()),
        ..Script::plain()
    };
    let memberships = CaseMemberships::allowed(ALLOWING_MEMBER);
    let admission = admission(&memberships, &context);
    check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert_eq!(script.events(), ["caller_auth", "caller_team"]);
    assert_eq!(memberships.asked(), ["caller-1:team-1"]);
}

#[tokio::test]
async fn caller_team_groups_grant_model() {
    let context = context();
    // the caller team's models deny the request, so only its access group can grant it
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(GROUP_TEAM.to_owned()),
        ..Script::plain()
    };
    let directory = Arc::new(GroupDirectory::new("ag-caller", &["gpt-4o"]));
    let lookup = team_lookup(&directory);
    let memberships = CaseMemberships::allowed(ALLOWING_MEMBER);
    let admission = admission(&memberships, &context);
    check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: Some(&lookup),
            admission: &admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert_eq!(script.events(), ["caller_auth", "caller_team"]);
    assert_eq!(memberships.asked(), ["caller-1:team-1"]);
}

#[tokio::test]
async fn caller_team_denial_stops_before_member() {
    let context = context();
    // a team denial ends the check without asking the member source or the user
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(DENYING_TEAM.to_owned()),
        ..Script::plain()
    };
    let memberships = CaseMemberships::allowed(ALLOWING_MEMBER);
    let admission = admission(&memberships, &context);
    let error = check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
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
    assert!(!script.events().contains(&"caller_user".to_owned()));
}

#[tokio::test]
async fn caller_member_denial_after_team_grant() {
    let context = context();
    // the member scope denies when the team grant passes but the member allowlist does not
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        team: Some(ALLOWING_TEAM.to_owned()),
        ..Script::plain()
    };
    let memberships = CaseMemberships::allowed(DENYING_MEMBER);
    let admission = admission(&memberships, &context);
    let error = check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
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
        ..Script::plain()
    };
    let memberships = CaseMemberships::allowed(ALLOWING_MEMBER);
    let context = context();
    let admission = admission(&memberships, &context);
    check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
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
        ..Script::plain()
    };
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        script.events(),
        ["caller_auth", "caller_team", "caller_user"]
    );
}

#[tokio::test]
async fn caller_user_denial_stops() {
    let context = context();
    let script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        user: Some(r#"{"user_id":"caller-1","models":["other-model"]}"#.to_owned()),
        ..Script::plain()
    };
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    let error = check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
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
        ..Script::plain()
    };
    let memberships = CaseMemberships::default();
    let context = context();
    let admission = admission(&memberships, &context);
    check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
    )
    .await
    .unwrap();
    assert!(script.events().contains(&"caller_user".to_owned()));
}

#[derive(Clone, Copy, Debug)]
enum CallerLookup {
    Auth,
    Team,
    User,
}

#[rstest]
#[case::caller_auth(CallerLookup::Auth)]
#[case::caller_team(CallerLookup::Team)]
#[case::caller_user(CallerLookup::User)]
#[tokio::test]
async fn caller_lookup_error_propagates(#[case] step: CallerLookup) {
    let context = context();
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    let script = match step {
        CallerLookup::Auth => Script {
            fails: vec![Failing::Caller],
            ..Script::plain()
        },
        CallerLookup::Team => Script {
            caller: Some(CALLER_KEY.to_owned()),
            fails: vec![Failing::Team],
            ..Script::plain()
        },
        CallerLookup::User => Script {
            caller: Some(CALLER_KEY.to_owned()),
            fails: vec![Failing::User],
            ..Script::plain()
        },
    };
    let error = check_agent_caller(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &CallerChecks {
            teams: None,
            admission: &admission,
            context: &context,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, AgentModelGrantsError::Caller(_)));
}

#[tokio::test]
async fn missing_source_fails_callers() {
    let context = context();
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    assert!(matches!(
        check_agent_caller(
            &one("gpt-4o"),
            Some(&agent_token(AGENT_KEY)),
            None,
            &CallerChecks {
                teams: None,
                admission: &admission,
                context: &context,
            },
        )
        .await,
        Err(AgentModelGrantsError::Unavailable)
    ));
}
