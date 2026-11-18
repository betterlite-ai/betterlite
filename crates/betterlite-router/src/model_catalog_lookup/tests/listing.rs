use super::*;

#[test]
fn clear_all_metadata() {
    let slots = Slots::filled();
    let caches = slots.caches();
    caches.invalidate(Invalidation::ModelMetadata);
    assert!(slots.info.0.lock().unwrap().is_none());
    assert!(slots.deployment.0.lock().unwrap().is_none());
    assert!(slots.zero_cost.0.lock().unwrap().is_none());
    assert!(slots.rows.0.lock().unwrap().is_none());
    assert_eq!(
        slots.access.0.lock().unwrap().clone(),
        Some("stale".to_owned())
    );
    let mut catalog = catalog(&[priced("gpt-4o", "a", "1e-6")]);
    *slots.zero_cost.0.lock().unwrap() = Some(true);
    rebuild_ids(&mut catalog, vec![priced("gpt-4o", "a", "2e-6")], &caches).unwrap();
    assert!(slots.zero_cost.0.lock().unwrap().is_none());
}

#[test]
fn clear_access_only() {
    let slots = Slots::filled();
    slots.caches().invalidate(Invalidation::AccessGroups);
    assert!(slots.access.0.lock().unwrap().is_none());
    assert_eq!(
        slots.info.0.lock().unwrap().clone(),
        Some("stale".to_owned())
    );
    assert_eq!(
        slots.deployment.0.lock().unwrap().clone(),
        Some("stale".to_owned())
    );
    assert_eq!(slots.zero_cost.0.lock().unwrap().clone(), Some(true));
    assert_eq!(
        slots.rows.0.lock().unwrap().clone(),
        Some("stale".to_owned())
    );
}

#[test]
fn resolve_model_id() {
    let snapshot = Snapshot::new(&[
        named("shared", "x", "openai/first"),
        named("other", "shared", "openai/other"),
        deployment("", "openai/second", object(vec![("id", text("id"))])),
        named("provider-model", "p", "vertex_ai/veo-2.0-generate-001"),
    ]);
    let lookup = snapshot.lookup();
    assert_eq!(
        lookup.resolve_model_name(Some("shared")).unwrap(),
        Some("shared".to_owned())
    );
    assert_eq!(
        lookup.resolve_model_name(Some("p")).unwrap(),
        Some("provider-model".to_owned())
    );
    assert_eq!(
        lookup.resolve_model_name(Some("id")).unwrap(),
        Some("id".to_owned())
    );
    assert_eq!(lookup.resolve_model_name(None).unwrap(), None);
    assert_eq!(lookup.resolve_model_name(Some("nope")).unwrap(), None);
}

#[test]
fn resolve_provider_model() {
    let snapshot = Snapshot::new(&[
        named("suffix-id", "s", "openai/prefix/veo-2.0-generate-001"),
        named("colon-model", "q", "openai/model:seq-9"),
        named("exact-model", "r", "plain"),
    ]);
    let lookup = snapshot.lookup();
    assert_eq!(
        lookup
            .resolve_model_name(Some("veo-2.0-generate-001"))
            .unwrap(),
        Some("suffix-id".to_owned())
    );
    assert_eq!(
        lookup.resolve_model_name(Some("seq-9")).unwrap(),
        Some("colon-model".to_owned())
    );
    assert_eq!(
        lookup.resolve_model_name(Some("plain")).unwrap(),
        Some("exact-model".to_owned())
    );
}

#[test]
fn candidate_routes() {
    let snapshot = Snapshot::new(&[
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("gpt-4o", "b", "azure/gpt-4o"),
        named("claude", "c", "anthropic/claude"),
    ])
    .with_alias("sonnet", AliasTarget::Name("gpt-4o".to_owned()))
    .with_group("chat", &["claude"]);
    let lookup = snapshot.lookup();
    assert_eq!(
        lookup.candidate_model_ids("sonnet", None).unwrap(),
        BTreeSet::from(["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(
        lookup.candidate_model_ids("chat", None).unwrap(),
        BTreeSet::from(["c".to_owned()])
    );
    assert_eq!(
        lookup.candidate_model_ids("gpt-4o", None).unwrap(),
        BTreeSet::from(["a".to_owned(), "b".to_owned()])
    );
}

// An item alias resolves through its model field, and a blank one falls back to the request name.
#[test]
fn candidate_item_alias() {
    let snapshot = Snapshot::new(&[named("gpt-4o", "a", "openai/gpt-4o")])
        .with_alias(
            "sonnet",
            AliasTarget::Item {
                model: Some(text("gpt-4o")),
                hidden: None,
            },
        )
        .with_alias(
            "blank",
            AliasTarget::Item {
                model: Some(text("")),
                hidden: Some(Json::Bool(false)),
            },
        );
    let lookup = snapshot.lookup();
    assert_eq!(
        lookup.candidate_model_ids("sonnet", None).unwrap(),
        BTreeSet::from(["a".to_owned()])
    );
    assert!(
        lookup
            .candidate_model_ids("blank", None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn candidate_route_fallbacks() {
    let snapshot = Snapshot::new(&[
        deployment("openai/*", "openai/*", object(vec![("id", text("w"))])),
        raw("numbers", "{\"id\": 42}"),
        raw("weird", "{\"id\": [\"x\"]}"),
    ]);
    let lookup = snapshot.lookup();
    assert_eq!(
        lookup.candidate_model_ids("openai/gpt-4o", None).unwrap(),
        BTreeSet::from(["w".to_owned()])
    );
    assert_eq!(
        lookup.candidate_model_ids("numbers", None).unwrap(),
        BTreeSet::from(["42".to_owned()])
    );
    assert!(
        lookup
            .candidate_model_ids("weird", None)
            .unwrap()
            .is_empty()
    );
    // A team with no owned row of this name falls through to the patterns, it does not stop the route.
    assert_eq!(
        lookup
            .candidate_model_ids("openai/gpt-4o", Some("t1"))
            .unwrap(),
        BTreeSet::from(["w".to_owned()])
    );
}

#[test]
fn candidate_empty_group_stops_the_route() {
    let snapshot = Snapshot::new(&[deployment(
        "*",
        "openai/*",
        object(vec![("id", text("star"))]),
    )])
    .with_group("empty", &[]);
    let lookup = snapshot.lookup();
    assert!(
        lookup
            .candidate_model_ids("empty", None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        lookup.candidate_model_ids("other", None).unwrap(),
        BTreeSet::from(["star".to_owned()])
    );
}

#[test]
fn candidate_id_text() {
    let rows = vec![
        raw("numbers", "{\"id\": 42}"),
        raw("flags", "{\"id\": true}"),
        raw("lists", "{\"id\": [\"x\"]}"),
        raw("missing", "{}"),
        named("plain", "text", "openai/plain"),
    ];
    assert_eq!(
        deployment_ids(&rows),
        BTreeSet::from(["42".to_owned(), "True".to_owned(), "text".to_owned()])
    );
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
