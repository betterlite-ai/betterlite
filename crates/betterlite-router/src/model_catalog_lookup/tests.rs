use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::deployment_catalog::GroupAlias;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_catalog::scope::litellm_model;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::deployment_identity::stable_id::generate_model_id;
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget, from_config};
use crate::model_catalog_lookup::cache::{CacheSlot, CatalogCaches, Invalidation};
use crate::model_catalog_lookup::error::LookupError;
use crate::model_catalog_lookup::index::{deployment_ids, rebuild_ids, rebuild_names};
use crate::model_catalog_lookup::listing::{get_all_deployments, should_include_deployment};
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::routing_groups::group::RoutingGroup;
use crate::routing_groups::index::RoutingGroupIndex;
use crate::wildcard_catalog::Candidate;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn deployment(name: &str, provider_model: &str, model_info: JsonObject) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", text(provider_model))]),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap_or_else(|error| panic!("the fixture deployment is invalid: {error}"))
}

fn named(name: &str, id: &str, provider_model: &str) -> DeploymentSpec {
    deployment(name, provider_model, object(vec![("id", text(id))]))
}

fn team_named(name: &str, id: &str, team: &str, public: &str) -> DeploymentSpec {
    deployment(
        name,
        "openai/team",
        object(vec![
            ("id", text(id)),
            ("team_id", text(team)),
            ("team_public_model_name", text(public)),
        ]),
    )
}

fn priced(name: &str, id: &str, rate: &str) -> DeploymentSpec {
    let params: JsonObject = serde_json::from_str(&format!(
        r#"{{"model":"openai/gpt-4o","input_cost_per_token":{rate}}}"#
    ))
    .unwrap();
    DeploymentSpec::new(
        name.to_owned(),
        params,
        object(vec![("id", text(id))]),
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn raw(name: &str, info: &str) -> DeploymentSpec {
    DeploymentSpec {
        model_name: name.to_owned(),
        litellm_params: object(vec![("model", text("openai/gpt-4o"))]),
        model_info: serde_json::from_str(info).unwrap(),
        extra: JsonObject::new(),
    }
}

fn catalog(rows: &[DeploymentSpec]) -> DeploymentCatalog {
    let mut catalog = DeploymentCatalog::default();
    for row in rows {
        catalog.model_names.insert(row.model_name.clone());
        catalog.index_patterns(row).unwrap();
        catalog.push(row.clone());
    }
    catalog
}

fn ids(rows: &[DeploymentSpec]) -> Vec<&str> {
    rows.iter().filter_map(DeploymentSpec::id).collect()
}

fn names(rows: &[DeploymentSpec]) -> Vec<&str> {
    rows.iter().map(|row| row.model_name.as_str()).collect()
}

fn provider_models(rows: &[DeploymentSpec]) -> Vec<&str> {
    rows.iter().map(|row| litellm_model(row)).collect()
}

struct Snapshot {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
    provider: Box<dyn Fn(&str) -> Option<String>>,
}

impl Snapshot {
    fn new(rows: &[DeploymentSpec]) -> Self {
        Self {
            catalog: catalog(rows),
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
            provider: Box::new(|_: &str| None),
        }
    }

    fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &*self.provider,
        }
    }

    fn with_alias(mut self, alias: &str, target: AliasTarget) -> Self {
        self.aliases.insert(alias.to_owned(), target);
        self
    }

    fn with_group(mut self, name: &str, members: &[&str]) -> Self {
        let group = RoutingGroup::new(
            name.to_owned(),
            members.iter().map(|member| (*member).to_owned()).collect(),
            "simple-shuffle",
            None,
            None,
        )
        .unwrap();
        self.groups = RoutingGroupIndex::build(vec![group], &BTreeSet::new(), &BTreeMap::new())
            .unwrap()
            .index;
        self
    }

    fn with_provider(mut self, provider: &str) -> Self {
        let resolved = provider.to_owned();
        self.provider = Box::new(move |_: &str| Some(resolved.clone()));
        self
    }
}

struct Slots {
    info: CacheSlot<String>,
    deployment: CacheSlot<String>,
    zero_cost: CacheSlot<bool>,
    rows: CacheSlot<String>,
    access: CacheSlot<String>,
}

impl Slots {
    fn filled() -> Self {
        let slots = Self {
            info: CacheSlot::default(),
            deployment: CacheSlot::default(),
            zero_cost: CacheSlot::default(),
            rows: CacheSlot::default(),
            access: CacheSlot::default(),
        };
        slots.info.store("stale".to_owned());
        slots.deployment.store("stale".to_owned());
        slots.zero_cost.store(true);
        slots.rows.store("stale".to_owned());
        slots.access.store("stale".to_owned());
        slots
    }

    fn caches(&self) -> CatalogCaches<'_> {
        CatalogCaches {
            model_group_info: &self.info,
            deployment_model_info: &self.deployment,
            zero_cost: &self.zero_cost,
            routing_group_rows: &self.rows,
            access_groups: &self.access,
        }
    }
}

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
    let first = lookup
        .get_deployment_by_model_group_name("gpt-4o")
        .unwrap()
        .unwrap();
    assert_eq!(first.id(), Some("a"));
    assert!(
        lookup
            .get_deployment_by_model_group_name("empty")
            .unwrap()
            .is_none()
    );
    assert!(
        lookup
            .get_deployment_by_model_group_name("claude")
            .unwrap()
            .is_none()
    );
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

#[test]
fn unknown_id() {
    let snapshot = Snapshot::new(&[named("gpt-4o", "a", "openai/gpt-4o")]);
    let lookup = snapshot.lookup();
    assert!(lookup.get_deployment("missing").unwrap().is_none());
    assert!(
        lookup
            .get_deployment_by_model_group_name("missing")
            .unwrap()
            .is_none()
    );
    assert!(!lookup.has_model_id("missing"));
    assert!(lookup.has_model_id("a"));
    assert!(lookup.get_model_ids(Some("missing"), false).is_empty());
    assert_eq!(lookup.resolve_model_name_from_model_id(None).unwrap(), None);
    assert_eq!(
        lookup.resolve_model_name_from_model_id(Some("")).unwrap(),
        None
    );
    assert_eq!(
        lookup
            .resolve_model_name_from_model_id(Some("missing"))
            .unwrap(),
        None
    );
}

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
    let direct = lookup
        .get_model_list_from_model_alias(Some("sonnet"))
        .unwrap();
    assert_eq!(ids(&direct), vec!["a"]);
    assert_eq!(names(&direct), vec!["sonnet"]);
    assert_eq!(
        snapshot.catalog.models.first().unwrap().model_name,
        "gpt-4o"
    );
    let hidden = lookup
        .get_model_list_from_model_alias(Some("secret"))
        .unwrap();
    assert_eq!(ids(&hidden), vec!["c"]);
    let everything = lookup.get_model_list_from_model_alias(None).unwrap();
    assert_eq!(ids(&everything), vec!["a"]);
    assert!(
        lookup
            .get_model_list_from_model_alias(Some("missing"))
            .unwrap()
            .is_empty()
    );
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
        no_hidden
            .lookup()
            .get_model_list_from_model_alias(Some("partial")),
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
        no_model.lookup().get_model_list_from_model_alias(None),
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
    assert!(caches.info.read().is_none());
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
        lookup.get_deployment_by_model_group_name("kept"),
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
    assert!(caches.access.read().is_none());
}

#[test]
fn clear_all_metadata() {
    let slots = Slots::filled();
    let caches = slots.caches();
    caches.invalidate(Invalidation::ModelMetadata);
    assert!(slots.info.read().is_none());
    assert!(slots.deployment.read().is_none());
    assert!(slots.zero_cost.read().is_none());
    assert!(slots.rows.read().is_none());
    assert_eq!(slots.access.read(), Some("stale".to_owned()));
    let mut catalog = catalog(&[priced("gpt-4o", "a", "1e-6")]);
    slots.zero_cost.store(true);
    rebuild_ids(&mut catalog, vec![priced("gpt-4o", "a", "2e-6")], &caches).unwrap();
    assert!(slots.zero_cost.read().is_none());
}

#[test]
fn clear_access_only() {
    let slots = Slots::filled();
    slots.caches().invalidate(Invalidation::AccessGroups);
    assert!(slots.access.read().is_none());
    assert_eq!(slots.info.read(), Some("stale".to_owned()));
    assert_eq!(slots.deployment.read(), Some("stale".to_owned()));
    assert_eq!(slots.zero_cost.read(), Some(true));
    assert_eq!(slots.rows.read(), Some("stale".to_owned()));
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
        lookup
            .resolve_model_name_from_model_id(Some("shared"))
            .unwrap(),
        Some("shared".to_owned())
    );
    assert_eq!(
        lookup.resolve_model_name_from_model_id(Some("p")).unwrap(),
        Some("provider-model".to_owned())
    );
    assert_eq!(
        lookup.resolve_model_name_from_model_id(Some("id")).unwrap(),
        Some("id".to_owned())
    );
    assert_eq!(lookup.resolve_model_name_from_model_id(None).unwrap(), None);
    assert_eq!(
        lookup
            .resolve_model_name_from_model_id(Some("nope"))
            .unwrap(),
        None
    );
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
            .resolve_model_name_from_model_id(Some("veo-2.0-generate-001"))
            .unwrap(),
        Some("suffix-id".to_owned())
    );
    assert_eq!(
        lookup
            .resolve_model_name_from_model_id(Some("seq-9"))
            .unwrap(),
        Some("colon-model".to_owned())
    );
    assert_eq!(
        lookup
            .resolve_model_name_from_model_id(Some("plain"))
            .unwrap(),
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
        lookup
            .get_candidate_model_ids_for_route("sonnet", None)
            .unwrap(),
        BTreeSet::from(["a".to_owned(), "b".to_owned()])
    );
    assert_eq!(
        lookup
            .get_candidate_model_ids_for_route("chat", None)
            .unwrap(),
        BTreeSet::from(["c".to_owned()])
    );
    assert_eq!(
        lookup
            .get_candidate_model_ids_for_route("gpt-4o", None)
            .unwrap(),
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
        lookup
            .get_candidate_model_ids_for_route("sonnet", None)
            .unwrap(),
        BTreeSet::from(["a".to_owned()])
    );
    assert!(
        lookup
            .get_candidate_model_ids_for_route("blank", None)
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
        lookup
            .get_candidate_model_ids_for_route("openai/gpt-4o", None)
            .unwrap(),
        BTreeSet::from(["w".to_owned()])
    );
    assert_eq!(
        lookup
            .get_candidate_model_ids_for_route("numbers", None)
            .unwrap(),
        BTreeSet::from(["42".to_owned()])
    );
    assert!(
        lookup
            .get_candidate_model_ids_for_route("weird", None)
            .unwrap()
            .is_empty()
    );
    // A team with no owned row of this name falls through to the patterns, it does not stop the route.
    assert_eq!(
        lookup
            .get_candidate_model_ids_for_route("openai/gpt-4o", Some("t1"))
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
            .get_candidate_model_ids_for_route("empty", None)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        lookup
            .get_candidate_model_ids_for_route("other", None)
            .unwrap(),
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
        Err(LookupError::UnindexedCandidate {
            model_id: Some(id)
        }) if id == "ghost"
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
    let aliased = lookup
        .get_model_list_from_model_alias(Some("assistant"))
        .unwrap();
    assert_eq!(ids(&aliased), vec!["a", "c"]);
    assert_eq!(names(&aliased), vec!["assistant", "assistant"]);
    assert!(
        lookup
            .get_model_list_from_model_alias(Some("chat"))
            .unwrap()
            .is_empty()
    );
    let group_rows = lookup.get_model_list(Some("chat"), None).unwrap();
    assert_eq!(ids(&group_rows), vec!["a", "c"]);
    assert_eq!(names(&group_rows), vec!["chat", "chat"]);
    let everything = lookup.get_model_list(None, None).unwrap();
    assert_eq!(ids(&everything), vec!["a", "c", "a", "c", "a", "c"]);
}
