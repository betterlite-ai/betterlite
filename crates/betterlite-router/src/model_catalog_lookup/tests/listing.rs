use super::*;

#[test]
fn multiple_deployments() {
    let snapshot = Snapshot::new(&[
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("gpt-4o", "b", "azure/gpt-4o"),
        named("claude", "a", "anthropic/claude"),
    ]);
    let lookup = snapshot.lookup();
    let siblings = lookup.get_model_list(Some("gpt-4o"), None).unwrap();
    assert_eq!(ids(&siblings), vec!["a", "b"]);
    assert_eq!(
        lookup.get_model_ids(Some("gpt-4o"), false),
        vec![text("a"), text("b")]
    );
    let everything = lookup.get_model_list(None, None).unwrap();
    assert_eq!(ids(&everything), vec!["a", "b", "a"]);
    assert_eq!(
        lookup.get_model_ids(None, false),
        vec![text("a"), text("b")]
    );
    assert_eq!(lookup.get_model_ids(None, true), vec![text("a"), text("b")]);
    assert_eq!(
        lookup.get_deployment("a").unwrap().unwrap().model_name,
        "claude"
    );
}

#[test]
fn alias_model_list() {
    let snapshot = Snapshot::new(&[
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("claude", "c", "anthropic/claude"),
    ])
    .with_alias("sonnet", AliasTarget::Name("gpt-4o".to_owned()))
    .with_alias(
        "secret",
        AliasTarget::Item {
            model: Some(text("claude")),
            hidden: Some(Json::Bool(true)),
        },
    )
    .with_alias("broken", AliasTarget::Unsupported(Json::Bool(false)));
    let lookup = snapshot.lookup();
    let direct = lookup.list_from_alias(Some("sonnet")).unwrap();
    assert_eq!(ids(&direct), vec!["a"]);
    assert_eq!(names(&direct), vec!["sonnet"]);
    assert_eq!(
        snapshot.catalog.models.first().unwrap().model_name,
        "gpt-4o"
    );
    let hidden = lookup.list_from_alias(Some("secret")).unwrap();
    assert_eq!(ids(&hidden), vec!["c"]);
    let everything = lookup.list_from_alias(None).unwrap();
    assert_eq!(ids(&everything), vec!["a"]);
    assert!(lookup.list_from_alias(Some("missing")).unwrap().is_empty());
}

#[test]
fn alias_from_config() {
    let aliases = IndexMap::from([(
        "sonnet".to_owned(),
        GroupAlias {
            model: "gpt-4o".to_owned(),
            hidden: true,
        },
    )]);
    assert_eq!(
        from_config(&aliases).get("sonnet"),
        Some(&AliasTarget::Item {
            model: Some(text("gpt-4o")),
            hidden: Some(Json::Bool(true)),
        })
    );
}

#[test]
fn malformed_aliases() {
    let no_hidden = Snapshot::new(&[]).with_alias(
        "partial",
        AliasTarget::Item {
            model: Some(text("gpt-4o")),
            hidden: None,
        },
    );
    assert!(matches!(
        no_hidden.lookup().list_from_alias(Some("partial")),
        Err(LookupError::MalformedAlias {
            field: "hidden",
            ..
        })
    ));
    let no_model = Snapshot::new(&[]).with_alias(
        "partial",
        AliasTarget::Item {
            model: None,
            hidden: Some(Json::Bool(false)),
        },
    );
    assert!(matches!(
        no_model.lookup().list_from_alias(None),
        Err(LookupError::MalformedAlias { field: "model", .. })
    ));
}

#[test]
fn include_deployment() {
    let global = named("gpt-4o", "g", "openai/gpt-4o");
    let empty_team = deployment(
        "gpt-4o",
        "openai/empty",
        object(vec![("id", text("e")), ("team_id", text(""))]),
    );
    let owned = deployment(
        "gpt-4o",
        "openai/owned",
        object(vec![("id", text("o")), ("team_id", text("t1"))]),
    );
    let snapshot = Snapshot::new(&[global, empty_team, owned]);
    let every = get_all_deployments(&snapshot.catalog, "gpt-4o", None, None);
    let empty_team_id = get_all_deployments(&snapshot.catalog, "gpt-4o", None, Some(""));
    let one_team = get_all_deployments(&snapshot.catalog, "gpt-4o", None, Some("t1"));
    assert_eq!(ids(&every), vec!["g", "e", "o"]);
    assert_eq!(ids(&empty_team_id), vec!["g", "e"]);
    assert_eq!(ids(&one_team), vec!["g", "o"]);
}

#[test]
fn include_deployment_scan() {
    let ghost = deployment(
        "internal",
        "openai/ghost",
        object(vec![
            ("id", text("s")),
            ("team_id", text("")),
            ("team_public_model_name", text("ghost")),
        ]),
    );
    let snapshot = Snapshot::new(&[ghost]);
    let found = get_all_deployments(&snapshot.catalog, "ghost", Some("alias"), Some(""));
    assert_eq!(ids(&found), vec!["s"]);
    assert_eq!(names(&found), vec!["alias"]);
    assert!(get_all_deployments(&snapshot.catalog, "ghost", None, Some("t1")).is_empty());
    assert!(get_all_deployments(&snapshot.catalog, "ghost", None, None).is_empty());
}

#[test]
fn wildcard_listing() {
    let snapshot = Snapshot::new(&[
        deployment("openai/*", "openai/*", object(vec![("id", text("w"))])),
        named("openai/gpt-4o", "e", "azure/exact"),
        deployment(
            "team-secret",
            "openai/team-*",
            object(vec![
                ("id", text("tw")),
                ("team_id", text("t1")),
                ("team_public_model_name", text("team/*")),
            ]),
        ),
    ]);
    let lookup = snapshot.lookup();
    let exact = lookup.get_model_list(Some("openai/gpt-4o"), None).unwrap();
    assert_eq!(ids(&exact), vec!["e"]);
    let wildcard = lookup.get_model_list(Some("openai/gpt-o1"), None).unwrap();
    assert_eq!(ids(&wildcard), vec!["w"]);
    assert_eq!(names(&wildcard), vec!["openai/gpt-o1"]);
    assert_eq!(provider_models(&wildcard), vec!["openai/gpt-o1"]);
    let team_rows = lookup
        .get_model_list(Some("team/alpha"), Some("t1"))
        .unwrap();
    assert_eq!(ids(&team_rows), vec!["tw"]);
    assert_eq!(provider_models(&team_rows), vec!["openai/team-alpha"]);
    let without_team = lookup.get_model_list(Some("team/alpha"), None).unwrap();
    assert!(without_team.is_empty());
}

#[test]
fn empty_team_wildcard_listing() {
    let snapshot = Snapshot::new(&[deployment(
        "team-secret",
        "openai/team-*",
        object(vec![
            ("id", text("tw")),
            ("team_id", text("")),
            ("team_public_model_name", text("team/*")),
        ]),
    )]);
    let lookup = snapshot.lookup();
    let team_rows = lookup.get_model_list(Some("team/alpha"), Some("")).unwrap();
    assert_eq!(ids(&team_rows), vec!["tw"]);
    assert_eq!(provider_models(&team_rows), vec!["openai/team-alpha"]);
}

#[test]
fn wildcard_provider_fallback() {
    let snapshot = Snapshot::new(&[deployment(
        "openai/*",
        "openai/*",
        object(vec![("id", text("w"))]),
    )])
    .with_provider("openai");
    let lookup = snapshot.lookup();
    let found = lookup.get_model_list(Some("gpt-4o"), None).unwrap();
    assert_eq!(ids(&found), vec!["w"]);
    assert_eq!(provider_models(&found), vec!["openai/gpt-4o"]);
}

#[test]
fn unindexed_candidate() {
    let mut snapshot = Snapshot::new(&[]);
    snapshot
        .catalog
        .pattern_router
        .add_pattern(
            "openai/*",
            Candidate {
                model_id: Some("ghost".to_owned()),
                model: "openai/gpt-4o".to_owned(),
            },
        )
        .unwrap();
    let lookup = snapshot.lookup();
    assert!(matches!(
        lookup.get_model_list(Some("openai/gpt-4o"), None),
        Err(LookupError::Route(CatalogLookupError::UnindexedCandidate {
            model_id: Some(id)
        })) if id == "ghost"
    ));
}

#[test]
fn alias_routing_group() {
    let snapshot = Snapshot::new(&[
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("claude", "c", "anthropic/claude"),
    ])
    .with_group("chat", &["gpt-4o", "claude"])
    .with_alias("assistant", AliasTarget::Name("chat".to_owned()));
    let lookup = snapshot.lookup();
    let aliased = lookup.list_from_alias(Some("assistant")).unwrap();
    assert_eq!(ids(&aliased), vec!["a", "c"]);
    assert_eq!(names(&aliased), vec!["assistant", "assistant"]);
    assert!(lookup.list_from_alias(Some("chat")).unwrap().is_empty());
    let group_rows = lookup.get_model_list(Some("chat"), None).unwrap();
    assert_eq!(ids(&group_rows), vec!["a", "c"]);
    assert_eq!(names(&group_rows), vec!["chat", "chat"]);
    let everything = lookup.get_model_list(None, None).unwrap();
    assert_eq!(ids(&everything), vec!["a", "c", "a", "c", "a", "c"]);
}
