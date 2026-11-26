use crate::auth_model_grants::caller::check_agent_caller_model_access;
use crate::auth_model_grants::ceiling::check_agent_access_group_model_access;
use crate::auth_model_grants::error::AgentModelGrantsError;
use crate::auth_model_patterns::denial::model_denied_client_message;
use crate::auth_request_models::RequestedModels;
use crate::entity_model_access::error::EntityModelAccessError;

use super::support::{
    AGENT_ALIAS_KEY, AGENT_KEY, AGENT_MODEL_ALIAS_KEY, AGENT_TEAM_KEY, ALLOWING_MEMBER,
    ALLOWING_TEAM, CALLER_KEY, CaseMemberships, DENYING_MEMBER, DENYING_TEAM, POLICY, Script,
    admission, agent_token, ceiling, context, denial_of, one, shallow_context, token,
};

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
async fn caller_falsey_model_and_token_skip() {
    let context = context();
    let memberships = CaseMemberships::default();
    let admission = admission(&memberships, &context);
    // the token carries a caller identity, so only the falsey model can skip before the lookup
    let denying = Script {
        caller: Some(CALLER_KEY.to_owned()),
        user: Some(r#"{"user_id":"caller-1","models":["other-model"]}"#.to_owned()),
        ..Script::default()
    };
    for model in [
        RequestedModels::None,
        RequestedModels::One(String::new()),
        RequestedModels::Many(Vec::new()),
    ] {
        check_agent_caller_model_access(
            &model,
            Some(&agent_token(AGENT_KEY)),
            Some(&denying),
            &admission,
            &context,
        )
        .await
        .unwrap();
    }
    check_agent_caller_model_access(&one("gpt-4o"), None, Some(&denying), &admission, &context)
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
    let script = Script::default();
    check_agent_caller_model_access(
        &one("gpt-4o"),
        Some(&agent_token(AGENT_KEY)),
        Some(&script),
        &admission,
        &context,
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
        ..Script::default()
    };
    check_agent_caller_model_access(
        &one("fast"),
        Some(&agent_token(AGENT_MODEL_ALIAS_KEY)),
        Some(&script),
        &caller_admission,
        &context,
    )
    .await
    .unwrap();
    assert_eq!(script.events(), ["caller_auth", "caller_team"]);

    // without a caller team the user's own models decide through the same alias map
    let user_script = Script {
        caller: Some(CALLER_KEY.to_owned()),
        user: Some(r#"{"user_id":"caller-1","models":["gpt-4o"]}"#.to_owned()),
        ..Script::default()
    };
    check_agent_caller_model_access(
        &one("fast"),
        Some(&agent_token(AGENT_MODEL_ALIAS_KEY)),
        Some(&user_script),
        &caller_admission,
        &context,
    )
    .await
    .unwrap();
    assert_eq!(
        user_script.events(),
        ["caller_auth", "caller_team", "caller_user"]
    );
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
