use super::*;

#[test]
fn rebuild_names() {
    let mut catalog = DeploymentCatalog::default();
    catalog.models = vec![
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("gpt-4o", "b", "azure/gpt-4o"),
        team_named("internal", "t", "t1", "team-gpt"),
        raw(
            "",
            "{\"id\": \"n\", \"team_id\": \"t2\", \"team_public_model_name\": \"team-two\"}",
        ),
    ];
    catalog.name_index.insert("stale".to_owned(), vec![9]);
    catalog
        .team_index
        .insert(("stale".to_owned(), "stale".to_owned()), vec![9]);
    catalog.id_index.insert("a".to_owned(), 0);
    rebuild_names(&mut catalog);
    assert_eq!(catalog.name_index.get("gpt-4o"), Some(&vec![0, 1]));
    assert_eq!(catalog.name_index.get("internal"), Some(&vec![2]));
    assert!(!catalog.name_index.contains_key("stale"));
    assert!(!catalog.name_index.contains_key(""));
    assert_eq!(
        catalog
            .team_index
            .get(&("t1".to_owned(), "team-gpt".to_owned())),
        Some(&vec![2])
    );
    assert_eq!(
        catalog
            .team_index
            .get(&("t2".to_owned(), "team-two".to_owned())),
        Some(&vec![3])
    );
    assert_eq!(catalog.id_index.get("a"), Some(&0));
    assert_eq!(catalog.models.len(), 4);
}

#[test]
fn generate_missing_id() {
    let mut catalog = DeploymentCatalog::default();
    let caches = Slots::filled();
    let params = object(vec![("model", text("openai/gpt-4o"))]);
    let rows = vec![
        raw("gpt-4o", "{}"),
        raw("", "{\"id\": null}"),
        DeploymentSpec {
            model_name: String::new(),
            litellm_params: JsonObject::new(),
            model_info: JsonObject::new(),
            extra: JsonObject::new(),
        },
    ];
    rebuild_ids(&mut catalog, rows, &caches.caches()).unwrap();
    let generated: Vec<&str> = catalog
        .models
        .iter()
        .filter_map(DeploymentSpec::id)
        .collect();
    assert_eq!(generated[0], generate_model_id("gpt-4o", &params));
    assert_eq!(generated[1], generate_model_id("", &params));
    assert_eq!(
        generated[2],
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(catalog.id_index.len(), 3);
    assert_eq!(catalog.name_index.get("gpt-4o"), Some(&vec![0]));
    assert!(caches.info.0.lock().unwrap().is_none());
}

#[test]
fn retain_explicit_empty_id() {
    let mut catalog = DeploymentCatalog::default();
    let caches = Slots::filled();
    let rows = vec![
        raw("gpt-4o", "{\"id\": \"\"}"),
        named("claude", "c", "anthropic/claude"),
    ];
    rebuild_ids(&mut catalog, rows, &caches.caches()).unwrap();
    assert_eq!(catalog.models.first().unwrap().id(), Some(""));
    assert!(catalog.id_index.contains_key(""));
    assert_eq!(catalog.id_index.get("c"), Some(&1));
}

#[test]
fn stale_map_precondition() {
    let mut catalog = DeploymentCatalog::default();
    catalog.id_index.insert("stale".to_owned(), 99);
    catalog.name_index.insert("kept".to_owned(), vec![7]);
    let caches = Slots::filled();
    rebuild_ids(
        &mut catalog,
        vec![named("gpt-4o", "a", "openai/gpt-4o")],
        &caches.caches(),
    )
    .unwrap();
    assert_eq!(catalog.id_index.get("stale"), Some(&99));
    assert_eq!(catalog.id_index.get("a"), Some(&0));
    assert_eq!(catalog.name_index.get("kept"), Some(&vec![7]));
    let snapshot = Snapshot {
        catalog,
        aliases: AliasMap::new(),
        groups: RoutingGroupIndex::default(),
        provider: Box::new(|_: &str| None),
    };
    let lookup = snapshot.lookup();
    assert!(matches!(
        lookup.get_deployment("stale"),
        Err(LookupError::InvalidRow { index: 99 })
    ));
    assert!(matches!(
        lookup.deployment_by_group_name("kept"),
        Err(LookupError::InvalidRow { index: 7 })
    ));
}

// A stale name index entry is skipped by the listing and the id projection, while the single-row lookup above still fails on it.
#[test]
fn stale_index_listing() {
    let mut snapshot = Snapshot::new(&[]);
    snapshot
        .catalog
        .name_index
        .insert("ghost".to_owned(), vec![7]);
    let lookup = snapshot.lookup();
    assert!(
        lookup
            .get_model_list(Some("ghost"), None)
            .unwrap()
            .is_empty()
    );
    assert!(lookup.get_model_ids(Some("ghost"), false).is_empty());
}

#[test]
fn rebuild_partial_failure() {
    let mut catalog = DeploymentCatalog::default();
    let caches = Slots::filled();
    let rows = vec![
        named("gpt-4o", "a", "openai/gpt-4o"),
        raw("bad", "{\"id\": [\"x\"]}"),
        named("claude", "c", "anthropic/claude"),
    ];
    let outcome = rebuild_ids(&mut catalog, rows, &caches.caches());
    assert!(matches!(outcome, Err(LookupError::InvalidId { index: 1 })));
    assert_eq!(ids(&catalog.models), vec!["a"]);
    assert!(catalog.id_index.contains_key("a"));
    assert!(!catalog.id_index.contains_key("c"));
    assert!(caches.access.0.lock().unwrap().is_none());
}

// A wildcard row and a later team row that share an id stay two deployments: the wildcard result reads the row the pattern registered, not the row the id map last indexed.
#[test]
fn duplicate_wildcard_id() {
    let snapshot = Snapshot::new(&[
        deployment("openai/*", "openai/*", object(vec![("id", text("x"))])),
        deployment(
            "team-secret",
            "azure/secret",
            object(vec![("id", text("x")), ("team_id", text("t1"))]),
        ),
    ]);
    let lookup = snapshot.lookup();
    let rows = lookup
        .get_model_list(Some("openai/gpt-4o"), Some("t1"))
        .unwrap();
    assert_eq!(ids(&rows), vec!["x"]);
    assert_eq!(names(&rows), vec!["openai/gpt-4o"]);
    assert_eq!(provider_models(&rows), vec!["openai/gpt-4o"]);
    assert_eq!(rows[0].model_info.get("team_id"), None);
    assert_eq!(
        lookup
            .candidate_model_ids("openai/gpt-4o", Some("t1"))
            .unwrap(),
        BTreeSet::from(["x".to_owned()])
    );
}

// Two rows registered under one pattern keep their own model_info and their registration order when they share an id.
#[test]
fn duplicate_pattern_row() {
    let snapshot = Snapshot::new(&[
        deployment(
            "openai/*",
            "openai/*",
            object(vec![("id", text("x")), ("team_id", text("t9"))]),
        ),
        deployment("openai/*", "openai/*", object(vec![("id", text("x"))])),
    ]);
    let rows = snapshot
        .lookup()
        .get_model_list(Some("openai/gpt-4o"), None)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].model_info.get("team_id"), Some(&text("t9")));
    assert_eq!(rows[1].model_info.get("team_id"), None);
}

// The scope decides which rows a wildcard may read: a row outside it that shares the id is not a candidate.
#[test]
fn pattern_scope_rows() {
    let snapshot = Snapshot::new(&[
        deployment(
            "team-secret",
            "azure/secret",
            object(vec![("id", text("x")), ("team_id", text("t1"))]),
        ),
        deployment("openai/*", "openai/*", object(vec![("id", text("x"))])),
        deployment(
            "team-plain",
            "openai/plain",
            object(vec![
                ("id", text("y")),
                ("team_id", text("t1")),
                ("team_public_model_name", text("plain")),
            ]),
        ),
        deployment(
            "team-star",
            "openai/team-*",
            object(vec![
                ("id", text("y")),
                ("team_id", text("t1")),
                ("team_public_model_name", text("team/*")),
            ]),
        ),
    ]);
    let lookup = snapshot.lookup();
    let wildcard = lookup.get_model_list(Some("openai/gpt-4o"), None).unwrap();
    assert_eq!(provider_models(&wildcard), vec!["openai/gpt-4o"]);
    assert_eq!(wildcard[0].model_info.get("team_id"), None);
    let team_rows = lookup
        .get_model_list(Some("team/alpha"), Some("t1"))
        .unwrap();
    assert_eq!(team_rows.len(), 1);
    assert_eq!(
        team_rows[0].model_info.get("team_public_model_name"),
        Some(&text("team/*"))
    );
}

// Each wildcard candidate resolves to the row registered with its id, not to a row at the candidate's position.
#[test]
fn pattern_row_by_id() {
    let snapshot = Snapshot::new(&[
        deployment("openai/*", "openai/*", object(vec![("id", text("a"))])),
        deployment("openai/*", "openai/second", object(vec![("id", text("b"))])),
    ]);
    let rows = snapshot
        .lookup()
        .get_model_list(Some("openai/gpt-4o"), None)
        .unwrap();
    assert_eq!(ids(&rows), vec!["a", "b"]);
}

// Removing the indexed row of a duplicated id drops the map entry, so the unfiltered ids are empty while the surviving row still answers its own model name.
#[test]
fn removed_duplicate_id() {
    let mut catalog = catalog(&[
        named("gpt-4o", "dup", "openai/gpt-4o"),
        named("claude", "dup", "anthropic/claude"),
    ]);
    assert!(catalog.remove("dup", 1).is_some());
    let snapshot = Snapshot {
        catalog,
        aliases: AliasMap::new(),
        groups: RoutingGroupIndex::default(),
        provider: Box::new(|_: &str| None),
    };
    let lookup = snapshot.lookup();
    assert!(lookup.get_model_ids(None, false).is_empty());
    assert_eq!(
        lookup.get_model_ids(Some("gpt-4o"), false),
        vec![text("dup")]
    );
}

// The unfiltered ids follow the id map, not the model list: a rebuild in the other order keeps the map's answer.
#[test]
fn reordered_rebuild() {
    let mut catalog = catalog(&[
        named("gpt-4o", "aaa", "openai/gpt-4o"),
        named("claude", "zzz", "anthropic/claude"),
    ]);
    let reversed = vec![
        named("claude", "zzz", "anthropic/claude"),
        named("gpt-4o", "aaa", "openai/gpt-4o"),
    ];
    let slots = Slots::filled();
    rebuild_ids(&mut catalog, reversed, &slots.caches()).unwrap();
    assert_eq!(names(&catalog.models), vec!["claude", "gpt-4o"]);
    let snapshot = Snapshot {
        catalog,
        aliases: AliasMap::new(),
        groups: RoutingGroupIndex::default(),
        provider: Box::new(|_: &str| None),
    };
    assert_eq!(
        snapshot.lookup().get_model_ids(None, false),
        vec![text("aaa"), text("zzz")]
    );
}

// Two wildcard patterns that share an id keep their own rows: a request for the anthropic pattern reads the anthropic row, not the later openai row its id ends on.
#[test]
fn wildcard_row_across_patterns() {
    let snapshot = Snapshot::new(&[
        deployment(
            "anthropic/*",
            "anthropic/claude-*",
            object(vec![("id", text("x")), ("team_id", text("t1"))]),
        ),
        deployment("openai/*", "openai/*", object(vec![("id", text("x"))])),
    ]);
    let rows = snapshot
        .lookup()
        .get_model_list(Some("anthropic/claude-3"), None)
        .unwrap();
    assert_eq!(ids(&rows), vec!["x"]);
    assert_eq!(rows[0].model_info.get("team_id"), Some(&text("t1")));
    assert_eq!(provider_models(&rows), vec!["anthropic/claude-3"]);
}

// Removing an earlier row leaves each wildcard on its own deployment, not on whatever row now sits at the removed position.
#[test]
fn removed_row_keeps_patterns() {
    let mut catalog = catalog(&[
        named("plain", "p", "openai/plain"),
        deployment("openai/*", "openai/*", object(vec![("id", text("w"))])),
        deployment(
            "anthropic/*",
            "anthropic/*",
            object(vec![("id", text("v"))]),
        ),
    ]);
    assert!(catalog.remove("p", 0).is_some());
    let snapshot = Snapshot {
        catalog,
        aliases: AliasMap::new(),
        groups: RoutingGroupIndex::default(),
        provider: Box::new(|_: &str| None),
    };
    let rows = snapshot
        .lookup()
        .get_model_list(Some("openai/gpt-4o"), None)
        .unwrap();
    assert_eq!(ids(&rows), vec!["w"]);
    assert_eq!(provider_models(&rows), vec!["openai/gpt-4o"]);
}

// A wildcard row whose litellm params carry the credential its result must keep.
fn wildcard(name: &str, id: &str, key: &str) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", text(name)), ("api_key", text(key))]),
        object(vec![("id", text(id))]),
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

// A rebuild in another order keeps each wildcard on the deployment it was registered from.
#[test]
fn reordered_rebuild_keeps_wildcard_rows() {
    let mut catalog = catalog(&[
        wildcard("openai/*", "o", "key-o"),
        wildcard("anthropic/*", "a", "key-a"),
    ]);
    let rows = vec![
        wildcard("anthropic/*", "a", "key-a"),
        wildcard("openai/*", "o", "key-o"),
    ];
    let slots = Slots::filled();
    rebuild_ids(&mut catalog, rows, &slots.caches()).unwrap();
    let snapshot = Snapshot {
        catalog,
        aliases: AliasMap::new(),
        groups: RoutingGroupIndex::default(),
        provider: Box::new(|_: &str| None),
    };
    let found = snapshot
        .lookup()
        .get_model_list(Some("openai/gpt-4o"), None)
        .unwrap();
    assert_eq!(ids(&found), vec!["o"]);
    assert_eq!(provider_models(&found), vec!["openai/gpt-4o"]);
    assert_eq!(found[0].litellm_params.get("api_key"), Some(&text("key-o")));
}

// A rebuild that stops on a bad row serves the wildcard that made it in and refuses the dropped row instead of another.
#[test]
fn partial_rebuild_keeps_wildcard_rows() {
    let mut catalog = catalog(&[
        wildcard("openai/*", "o", "key-o"),
        wildcard("anthropic/*", "a", "key-a"),
    ]);
    let rows = vec![
        wildcard("openai/*", "o", "key-o"),
        raw("bad", "{\"id\": [\"x\"]}"),
        wildcard("anthropic/*", "a", "key-a"),
    ];
    let slots = Slots::filled();
    let outcome = rebuild_ids(&mut catalog, rows, &slots.caches());
    assert!(matches!(outcome, Err(LookupError::InvalidId { index: 1 })));
    let snapshot = Snapshot {
        catalog,
        aliases: AliasMap::new(),
        groups: RoutingGroupIndex::default(),
        provider: Box::new(|_: &str| None),
    };
    let lookup = snapshot.lookup();
    let found = lookup.get_model_list(Some("openai/gpt-4o"), None).unwrap();
    assert_eq!(ids(&found), vec!["o"]);
    assert_eq!(found[0].litellm_params.get("api_key"), Some(&text("key-o")));
    assert!(matches!(
        lookup.get_model_list(Some("anthropic/claude-3"), None),
        Err(LookupError::Route(CatalogLookupError::UnindexedCandidate {
            model_id: Some(id)
        })) if id == "a"
    ));
}
