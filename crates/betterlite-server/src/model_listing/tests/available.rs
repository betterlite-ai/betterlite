use super::*;

use axum::http::StatusCode;

use crate::model_listing::available::available_models_for_user;

#[tokio::test]
async fn a_requested_team_replaces_the_key_models_after_the_membership_check() {
    let key = key(r#"{"models": ["key-model"]}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["key-model", "team-model"]);
    let mut access = FakeAccess::default();
    access.member = true;
    access.team_row = Some(team(&["team-model"]));
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(
        &key,
        &listing_request(Some("t1"), listing_flags()),
        &sources,
    )
    .await
    .expect("the listing answers");
    assert_eq!(models, strings(&["team-model"]));
    assert_eq!(access.validated.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_requested_team_the_caller_is_not_on_fails_the_listing() {
    let key = key(r#"{"models": ["key-model"]}"#);
    let router = FakeRouter::default();
    let mut access = FakeAccess::default();
    access.team_row = Some(team(&["team-model"]));
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let error = available_models_for_user(
        &key,
        &listing_request(Some("t1"), listing_flags()),
        &sources,
    )
    .await
    .expect_err("the listing refuses");
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_requested_team_the_database_does_not_have_fails_the_listing() {
    let key = key(r#"{"models": ["key-model"]}"#);
    let router = FakeRouter::default();
    let mut access = FakeAccess::default();
    access.team_missing = true;
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let error = available_models_for_user(
        &key,
        &listing_request(Some("t1"), listing_flags()),
        &sources,
    )
    .await
    .expect_err("the listing refuses");
    assert_eq!(error.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_requested_team_without_a_database_lists_by_the_key() {
    let key = key(r#"{"models": ["key-model"]}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["key-model", "team-model"]);
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(
        &key,
        &listing_request(Some("t1"), listing_flags()),
        &sources,
    )
    .await
    .expect("the listing answers");
    assert_eq!(models, strings(&["key-model"]));
    assert_eq!(access.validated.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn key_access_group_models_extend_a_restricted_key_list() {
    let key = key(r#"{"models": ["a"]}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a", "b"]);
    let mut access = FakeAccess::default();
    access.key_group_models = strings(&["b"]);
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["a", "b"]));
}

#[tokio::test]
async fn access_group_models_leave_an_unrestricted_key_alone() {
    let key = key(r#"{}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a"]);
    let mut access = FakeAccess::default();
    access.key_group_models = strings(&["ghost"]);
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["a"]));
}

#[tokio::test]
async fn team_access_group_models_come_before_the_key_groups_without_repeats() {
    let key = key(r#"{"models": ["a"]}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a", "t1", "dup", "k1"]);
    let mut access = FakeAccess::default();
    access.member = true;
    access.team_row = Some(team(&["a"]));
    access.team_group_models = strings(&["t1", "dup"]);
    access.key_group_models = strings(&["k1", "dup"]);
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(
        &key,
        &listing_request(Some("t1"), listing_flags()),
        &sources,
    )
    .await
    .expect("the listing answers");
    assert_eq!(models, strings(&["a", "t1", "dup", "k1"]));
    assert_eq!(access.team_reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_team_the_listing_cannot_fetch_still_lists_the_key_groups() {
    let key = key(r#"{"models": ["a"], "team_id": "t1"}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a", "from-key"]);
    let mut access = FakeAccess::default();
    access.team_missing = true;
    access.team_group_models = strings(&["from-team"]);
    access.key_group_models = strings(&["from-key"]);
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["a", "from-key"]));
}

#[tokio::test]
async fn the_effective_team_id_reaches_the_wildcard_lookup() {
    let key = key(r#"{"models": ["wild/*"], "team_id": "t1"}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["wild/*"]);
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, Vec::<String>::new());
    assert_eq!(seen_teams(&router), vec![Some("t1".to_owned())]);
}

#[tokio::test]
async fn a_requested_team_is_the_team_the_wildcard_lookup_sees() {
    let key = key(r#"{"models": ["wild/*"], "team_id": "t1"}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["wild/*"]);
    let mut access = FakeAccess::default();
    access.member = true;
    access.team_row = Some(team(&[]));
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(
        &key,
        &listing_request(Some("t2"), listing_flags()),
        &sources,
    )
    .await
    .expect("the listing answers");
    assert_eq!(models, Vec::<String>::new());
    assert_eq!(seen_teams(&router), vec![Some("t2".to_owned())]);
}

#[tokio::test]
async fn an_agent_ceiling_caps_the_list_and_keeps_its_order() {
    let key = key(r#"{"agent_id": "agent-1"}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a", "b", "c"]);
    let mut access = FakeAccess::default();
    access.ceiling = Some(BTreeSet::from(["a".to_owned(), "c".to_owned()]));
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["a", "c"]));
}

#[tokio::test]
async fn an_agent_without_a_ceiling_keeps_the_list() {
    let key = key(r#"{"agent_id": "agent-1"}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a", "b"]);
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["a", "b"]));
}

#[tokio::test]
async fn an_agent_key_without_a_router_is_capped_to_its_ceiling_models() {
    let key = key(r#"{"models": ["a", "b"], "agent_id": "agent-1"}"#);
    let mut access = FakeAccess::default();
    access.ceiling = Some(BTreeSet::from(["b".to_owned()]));
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(None, &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["b"]));
}

#[tokio::test]
async fn an_empty_ceiling_caps_the_list_to_nothing() {
    let key = key(r#"{"agent_id": "agent-1"}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a"]);
    let mut access = FakeAccess::default();
    access.ceiling = Some(BTreeSet::new());
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, Vec::<String>::new());
}

#[tokio::test]
async fn a_key_without_an_agent_id_is_not_capped() {
    let key = key(r#"{"agent_id": null}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a"]);
    let mut access = FakeAccess::default();
    access.ceiling = Some(BTreeSet::from(["other".to_owned()]));
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["a"]));
}

#[tokio::test]
async fn infer_model_from_keys_adds_the_provider_catalogue() {
    let key = key(r#"{}"#);
    let providers = StaticProviders(strings(&["inferred"]));
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(None, &access, &providers, &credentials);
    let request = AvailableModelsRequest {
        user_model: None,
        team_id: None,
        flags: ListingFlags {
            infer_model_from_keys: true,
            return_wildcard_routes: false,
            groups: AccessGroupScope {
                include: false,
                only: false,
            },
        },
    };
    let models = available_models_for_user(&key, &request, &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["inferred"]));
}

#[tokio::test]
async fn infer_model_from_keys_off_leaves_the_list_empty() {
    let key = key(r#"{}"#);
    let providers = StaticProviders(strings(&["inferred"]));
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(None, &access, &providers, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, Vec::<String>::new());
}

#[tokio::test]
async fn return_wildcard_routes_keeps_the_wildcard_name() {
    let key = key(r#"{}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["wild/*"]);
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let request = AvailableModelsRequest {
        user_model: None,
        team_id: None,
        flags: ListingFlags {
            infer_model_from_keys: false,
            return_wildcard_routes: true,
            groups: AccessGroupScope {
                include: false,
                only: false,
            },
        },
    };
    let models = available_models_for_user(&key, &request, &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["wild/*"]));
}

#[tokio::test]
async fn return_wildcard_routes_off_drops_the_wildcard_name() {
    let key = key(r#"{}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["wild/*"]);
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, Vec::<String>::new());
}

#[tokio::test]
async fn only_model_access_groups_keeps_the_group_names() {
    let key = key(r#"{"models": ["g1"]}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a"]);
    router
        .access_groups
        .insert("g1".to_owned(), strings(&["a"]));
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let request = AvailableModelsRequest {
        user_model: None,
        team_id: None,
        flags: ListingFlags {
            infer_model_from_keys: false,
            return_wildcard_routes: false,
            groups: AccessGroupScope {
                include: true,
                only: true,
            },
        },
    };
    let models = available_models_for_user(&key, &request, &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["g1"]));
}

#[tokio::test]
async fn non_text_grants_are_ignored() {
    let key = key(r#"{"models": ["a", 5, null, ["b"], {"c": 1}]}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["a", "b", "c"]);
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let models = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect("the listing answers");
    assert_eq!(models, strings(&["a"]));
}

#[tokio::test]
async fn a_failed_wildcard_lookup_fails_the_listing() {
    let key = key(r#"{}"#);
    let mut router = FakeRouter::default();
    router.model_names = strings(&["wild/*"]);
    router.rows_fail = true;
    let access = FakeAccess::default();
    let credentials = CredentialSnapshot::default();
    let sources = listing_sources(Some(&router), &access, &NoProviders, &credentials);
    let error = available_models_for_user(&key, &listing_request(None, listing_flags()), &sources)
        .await
        .expect_err("the expansion fails");
    assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
}
