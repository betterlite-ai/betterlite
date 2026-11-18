use super::*;

#[test]
fn lookup_id() {
    let snapshot = Snapshot::new(&[
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("gpt-4o", "b", "azure/gpt-4o"),
    ]);
    let lookup = snapshot.lookup();
    let second = lookup.get_deployment("b").unwrap().unwrap();
    assert_eq!(second.id(), Some("b"));
    assert_eq!(second.model_name, "gpt-4o");
    assert_eq!(
        second.litellm_params.get("model"),
        Some(&text("azure/gpt-4o"))
    );
}

#[test]
fn lookup_group() {
    let mut snapshot = Snapshot::new(&[
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("gpt-4o", "b", "azure/gpt-4o"),
    ]);
    snapshot
        .catalog
        .name_index
        .insert("empty".to_owned(), Vec::new());
    let lookup = snapshot.lookup();
    let first = lookup.deployment_by_group_name("gpt-4o").unwrap().unwrap();
    assert_eq!(first.id(), Some("a"));
    assert!(lookup.deployment_by_group_name("empty").unwrap().is_none());
    assert!(lookup.deployment_by_group_name("claude").unwrap().is_none());
}

#[test]
fn team_public_name() {
    let snapshot = Snapshot::new(&[
        named("gpt-4o", "g", "openai/gpt-4o"),
        team_named("internal", "t", "t1", "team-gpt"),
    ]);
    let lookup = snapshot.lookup();
    let owned = lookup.get_deployment("t").unwrap().unwrap();
    assert!(should_include_deployment("team-gpt", owned, Some("t1")));
    assert!(!should_include_deployment("team-gpt", owned, Some("t2")));
    assert!(!should_include_deployment("team-gpt", owned, None));
    assert!(should_include_deployment("internal", owned, Some("t1")));
    assert!(!should_include_deployment("internal", owned, Some("t2")));
    assert!(should_include_deployment("internal", owned, None));
    let pool = get_all_deployments(&snapshot.catalog, "team-gpt", None, Some("t1"));
    assert_eq!(ids(&pool), vec!["t"]);
    assert!(get_all_deployments(&snapshot.catalog, "team-gpt", None, None).is_empty());
    assert_eq!(
        lookup.get_model_names(Some("t1")).unwrap(),
        vec!["gpt-4o", "team-gpt"]
    );
    assert_eq!(lookup.get_model_names(Some("t2")).unwrap(), vec!["gpt-4o"]);
    assert_eq!(lookup.get_model_names(None).unwrap(), vec!["gpt-4o"]);
    assert_eq!(lookup.get_model_ids(None, true), vec![text("g")]);
    assert_eq!(
        lookup.get_model_ids(None, false),
        vec![text("g"), text("t")]
    );
}

#[test]
fn named_team_ids() {
    let snapshot = Snapshot::new(&[
        named("gpt-4o", "g", "openai/gpt-4o"),
        team_named("gpt-4o", "t", "t1", "team-gpt"),
    ]);
    let lookup = snapshot.lookup();
    assert_eq!(
        lookup.get_model_ids(Some("gpt-4o"), false),
        vec![text("g"), text("t")]
    );
    assert_eq!(lookup.get_model_ids(Some("gpt-4o"), true), vec![text("g")]);
}

// A global row with no model name lists as an empty name, in row order.
#[test]
fn unnamed_global_name() {
    let snapshot = Snapshot::new(&[
        raw("", "{\"id\": \"n\"}"),
        named("gpt-4o", "g", "openai/gpt-4o"),
    ]);
    assert_eq!(
        snapshot.lookup().get_model_names(None).unwrap(),
        vec!["", "gpt-4o"]
    );
}

#[test]
fn unknown_id() {
    let snapshot = Snapshot::new(&[named("gpt-4o", "a", "openai/gpt-4o")]);
    let lookup = snapshot.lookup();
    assert!(lookup.get_deployment("missing").unwrap().is_none());
    assert!(
        lookup
            .deployment_by_group_name("missing")
            .unwrap()
            .is_none()
    );
    assert!(lookup.get_model_ids(Some("missing"), false).is_empty());
    assert_eq!(lookup.resolve_model_name(None).unwrap(), None);
    assert_eq!(lookup.resolve_model_name(Some("")).unwrap(), None);
    assert_eq!(lookup.resolve_model_name(Some("missing")).unwrap(), None);
}

// The unfiltered ids follow the id map's insertion order, and replacing an id keeps the position of its first insertion.
#[test]
fn insertion_order_ids() {
    let mut catalog = catalog(&[
        named("gpt-4o", "z", "openai/gpt-4o"),
        named("claude", "a", "anthropic/claude"),
    ]);
    catalog.push(named("gpt-4o", "z", "openai/gpt-4o"));
    let snapshot = Snapshot {
        catalog,
        aliases: AliasMap::new(),
        groups: RoutingGroupIndex::default(),
        provider: Box::new(|_: &str| None),
    };
    assert_eq!(
        snapshot.lookup().get_model_ids(None, false),
        vec![text("z"), text("a")]
    );
}

// A row that lost its model_info.id after the router built it is skipped by both id listings, while its id stays a literal map member for the name resolution.
#[test]
fn missing_row_id() {
    let mut snapshot = Snapshot::new(&[
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("gpt-4o", "b", "azure/gpt-4o"),
    ]);
    snapshot.catalog.models[0].model_info.remove("id");
    let lookup = snapshot.lookup();
    assert_eq!(lookup.get_model_ids(Some("gpt-4o"), false), vec![text("b")]);
    assert_eq!(lookup.get_model_ids(None, false), vec![text("b")]);
    assert_eq!(
        lookup.resolve_model_name(Some("a")).unwrap(),
        Some("gpt-4o".to_owned())
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
    let snapshot = Snapshot::new(&[
        named("gpt-4o", "a", "openai/gpt-4o"),
        named("claude", "c", "anthropic/claude"),
    ])
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
    )
    .with_alias(
        "gpt-4o",
        AliasTarget::Item {
            model: Some(text("")),
            hidden: Some(Json::Bool(false)),
        },
    )
    .with_alias("claude", AliasTarget::Name(String::new()));
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
    assert_eq!(
        lookup.candidate_model_ids("gpt-4o", None).unwrap(),
        BTreeSet::from(["a".to_owned()])
    );
    assert_eq!(
        lookup.candidate_model_ids("claude", None).unwrap(),
        BTreeSet::from(["c".to_owned()])
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
