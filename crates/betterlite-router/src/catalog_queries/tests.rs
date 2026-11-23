#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use betterlite_config::sections::fallback_plans::Settings;
use betterlite_config::sections::fallback_plans::chains::{
    FallbackChainEntry, FallbackTarget, ParameterizedTarget,
};
use betterlite_config::sections::weighted_failover;
use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;
use rstest::rstest;
use tokio::runtime::Builder;

use crate::catalog_queries::CatalogQueries;
use crate::catalog_queries::messages::{MessagesRequest, RouterFallbacks};
use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::fallback_plans::attempts::AttemptedTargets;
use crate::fallback_plans::chains::ProviderNames;
use crate::fallback_plans::groups::{Bucket, MetadataValues, RequestMetadata};
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::model_group_info::sources::{
    DeploymentInfo, GroupSources, ProviderAnswer, ProviderFailure, ProviderRequest,
    ProviderResolver,
};
use crate::model_group_lookups::access_groups::AccessGroups;
use crate::model_group_lookups::group_info::GroupInfoCache;
use crate::routing_groups::group::RoutingGroup;
use crate::routing_groups::index::RoutingGroupIndex;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

const fn no_provider(_model: &str) -> Option<String> {
    None
}

fn deployment(name: &str, params: JsonObject, info: JsonObject) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        params,
        info,
        JsonObject::new(),
        String::new,
    )
    .expect("the fixture deployment is invalid")
}

fn plain(name: &str, id: &str, provider_model: &str) -> DeploymentSpec {
    deployment(
        name,
        object(vec![("model", text(provider_model))]),
        object(vec![("id", text(id))]),
    )
}

fn params_ordered(name: &str, id: &str, order: i64) -> DeploymentSpec {
    deployment(
        name,
        object(vec![("model", text("openai/m")), ("order", number(order))]),
        object(vec![("id", text(id))]),
    )
}

fn info_ordered(name: &str, id: &str, order: i64) -> DeploymentSpec {
    deployment(
        name,
        object(vec![("model", text("openai/m"))]),
        object(vec![("id", text(id)), ("order", number(order))]),
    )
}

fn team_row(name: &str, id: &str, team: &str, public: Option<&str>) -> DeploymentSpec {
    let mut info = vec![("id", text(id)), ("team_id", text(team))];
    if let Some(public_name) = public {
        info.push(("team_public_model_name", text(public_name)));
    }
    deployment(
        name,
        object(vec![("model", text("openai/m"))]),
        object(info),
    )
}

fn team_ordered(name: &str, id: &str, team: &str, public: &str, order: i64) -> DeploymentSpec {
    deployment(
        name,
        object(vec![("model", text("openai/m")), ("order", number(order))]),
        object(vec![
            ("id", text(id)),
            ("team_id", text(team)),
            ("team_public_model_name", text(public)),
        ]),
    )
}

fn grouped(name: &str, id: &str, groups: &[&str]) -> DeploymentSpec {
    let listed = groups.iter().map(|entry| text(entry)).collect();
    deployment(
        name,
        object(vec![("model", text("openai/m"))]),
        object(vec![
            ("id", text(id)),
            ("access_groups", Json::List(listed)),
        ]),
    )
}

// The snapshot one request's queries read, built the way the catalog updater builds it.
struct World {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
    access_groups: AccessGroups,
    group_info: GroupInfoCache,
}

impl World {
    fn new(rows: &[DeploymentSpec]) -> Self {
        let mut catalog = DeploymentCatalog::default();
        for row in rows {
            catalog.model_names.insert(row.model_name.clone());
            catalog.index_patterns(row).unwrap();
            catalog.push(row.clone());
        }
        Self {
            catalog,
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
            access_groups: AccessGroups::default(),
            group_info: GroupInfoCache::new(64),
        }
    }

    fn with_alias(mut self, name: &str, target: &str) -> Self {
        self.aliases
            .insert(name.to_owned(), AliasTarget::Name(target.to_owned()));
        self
    }

    fn with_group(mut self, group_name: &str, models: &[&str]) -> Self {
        let listed = models.iter().map(|entry| (*entry).to_owned()).collect();
        let group = RoutingGroup::new(group_name.to_owned(), listed, "simple-shuffle", None, None)
            .expect("the fixture routing group is invalid");
        let built = RoutingGroupIndex::build(vec![group], &BTreeSet::new(), &BTreeMap::new())
            .expect("the fixture routing group index is invalid");
        self.groups = built.index;
        self
    }

    fn queries(&self) -> CatalogQueries<'_> {
        CatalogQueries {
            lookup: CatalogLookup {
                catalog: &self.catalog,
                model_group_alias: &self.aliases,
                routing_groups: &self.groups,
                inferred_provider: &no_provider,
            },
            access_groups: &self.access_groups,
            group_info: &self.group_info,
        }
    }
}

fn metadata(selection: Option<&str>, group: Option<&str>) -> RequestMetadata {
    RequestMetadata {
        metadata: Bucket {
            present: true,
            values: MetadataValues {
                pre_routing_selected_model: selection.map(str::to_owned),
                model_group: group.map(str::to_owned),
                original_model_group: None,
                disable_fallbacks: false,
            },
        },
        litellm_metadata: Bucket::default(),
    }
}

// One Messages call's values, held so the request view can borrow them.
#[derive(Default)]
struct Call {
    metadata: RequestMetadata,
    team_id: Option<String>,
    target_order: Option<i64>,
    fallbacks: Option<Vec<FallbackChainEntry>>,
    content_policy: Option<Vec<FallbackChainEntry>>,
    disable: bool,
    attempted: AttemptedTargets,
}

impl Call {
    fn request(&self) -> MessagesRequest<'_> {
        MessagesRequest {
            metadata: &self.metadata,
            team_id: self.team_id.as_deref(),
            target_order: self.target_order,
            fallbacks: self.fallbacks.as_deref(),
            content_policy_fallbacks: self.content_policy.as_deref(),
            disable_fallbacks: self.disable,
            attempted_targets: &self.attempted,
        }
    }

    fn fallbacks(mut self, entries: Vec<FallbackChainEntry>) -> Self {
        self.fallbacks = Some(entries);
        self
    }

    fn content_policy(mut self, entries: Vec<FallbackChainEntry>) -> Self {
        self.content_policy = Some(entries);
        self
    }

    fn target(mut self, order: i64) -> Self {
        self.target_order = Some(order);
        self
    }

    fn tried(mut self, targets: &[FallbackTarget]) -> Self {
        self.attempted = AttemptedTargets::from_targets(targets);
        self
    }

    fn off(mut self) -> Self {
        self.disable = true;
        self
    }

    fn selecting(mut self, chosen: Option<&str>, group: Option<&str>) -> Self {
        self.metadata = metadata(chosen, group);
        self
    }

    fn team(mut self, team_id: &str) -> Self {
        self.team_id = Some(team_id.to_owned());
        self
    }
}

// The tables a fallback key resolves its provider names through.
struct Tables {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    custom_providers: BTreeSet<String>,
    environment: BTreeMap<String, String>,
    registry: JSONProviderRegistry,
}

impl Tables {
    fn load() -> Self {
        Self {
            catalog: ModelCatalog::parse(CATALOG.as_bytes(), UnknownFields::Reject)
                .expect("the fixture model catalog is invalid"),
            names: BTreeSet::new(),
            custom_providers: BTreeSet::new(),
            environment: BTreeMap::new(),
            registry: JSONProviderRegistry::load(),
        }
    }

    fn provider_names(&self) -> ProviderNames<'_> {
        ProviderNames {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.custom_providers,
            environment: &self.environment,
            registry: &self.registry,
        }
    }
}

const CATALOG: &str = r#"{"gpt-4": {"litellm_provider": "openai"}}"#;

// The router-level fallback values the check reads.
struct RouterConfig {
    settings: Settings,
    weighted: weighted_failover::Settings,
    tables: Tables,
}

impl RouterConfig {
    fn new(settings: Settings) -> Self {
        Self {
            settings,
            weighted: weighted_failover::Settings::default(),
            tables: Tables::load(),
        }
    }

    fn weighting(mut self, enabled: bool) -> Self {
        self.weighted = weighted_failover::Settings { enabled };
        self
    }

    fn fallbacks(&self) -> RouterFallbacks<'_> {
        RouterFallbacks {
            settings: &self.settings,
            weighted_failover: self.weighted,
            provider_names: self.tables.provider_names(),
        }
    }
}

fn plans(fallbacks: Vec<FallbackChainEntry>, content_policy: Vec<FallbackChainEntry>) -> Settings {
    Settings {
        fallbacks,
        context_window_fallbacks: Vec::new(),
        content_policy_fallbacks: content_policy,
        max_fallbacks: 5,
    }
}

fn name(target: &str) -> FallbackTarget {
    FallbackTarget::Model(target.to_owned())
}

fn group(name: &str, targets: Vec<FallbackTarget>) -> FallbackChainEntry {
    FallbackChainEntry::Group(IndexMap::from([(name.to_owned(), targets)]))
}

#[rstest]
#[case::a_model_name("gpt-4o")]
#[case::a_deployment_id("dep-a")]
#[case::an_alias("fast")]
#[case::a_routing_group("balanced")]
fn recognized_names_the_snapshot_serves(#[case] model: &str) {
    let world = World::new(&[plain("gpt-4o", "dep-a", "openai/gpt-4o")])
        .with_alias("fast", "gpt-4o")
        .with_group("balanced", &["gpt-4o"]);
    assert!(world.queries().is_recognized_model(model));
}

#[rstest]
#[case::an_unknown_name("other")]
#[case::a_name_a_wildcard_route_would_take("team-alpha")]
fn recognized_refuses_what_only_a_wildcard_route_would_serve(#[case] model: &str) {
    let world = World::new(&[
        plain("gpt-4o", "dep-a", "openai/gpt-4o"),
        plain("team-*", "dep-b", "openai/m"),
    ]);
    assert!(!world.queries().is_recognized_model(model));
}

#[test]
fn team_mapping_keeps_the_caller_name() {
    let world = World::new(&[team_row("internal-a", "dep-a", "team-1", Some("public-a"))]);
    let mapped = world
        .queries()
        .map_team_model(Some("internal-a"), "team-1")
        .unwrap();
    assert_eq!(mapped.as_deref(), Some("internal-a"));
}

#[test]
fn team_mapping_falls_back_to_the_public_name() {
    let world = World::new(&[
        plain("global", "dep-g", "openai/m"),
        team_row("internal-a", "dep-a", "team-1", Some("public-a")),
    ]);
    let mapped = world.queries().map_team_model(None, "team-1").unwrap();
    assert_eq!(mapped.as_deref(), Some("public-a"));
}

#[test]
fn team_mapping_names_the_deployment_without_a_public_name() {
    let world = World::new(&[team_row("internal-b", "dep-b", "team-2", None)]);
    let mapped = world.queries().map_team_model(None, "team-2").unwrap();
    assert_eq!(mapped.as_deref(), Some("internal-b"));
}

#[test]
fn team_mapping_ignores_an_empty_public_name() {
    let world = World::new(&[team_row("internal-c", "dep-c", "team-3", Some(""))]);
    let mapped = world.queries().map_team_model(None, "team-3").unwrap();
    assert_eq!(mapped.as_deref(), Some("internal-c"));
}

#[test]
fn team_mapping_is_none_without_a_team_row() {
    let world = World::new(&[plain("global", "dep-g", "openai/m")]);
    let mapped = world.queries().map_team_model(None, "team-1").unwrap();
    assert_eq!(mapped, None);
}

#[test]
fn team_mapping_is_none_without_deployments() {
    let world = World::new(&[]);
    let mapped = world
        .queries()
        .map_team_model(Some("public-a"), "team-1")
        .unwrap();
    assert_eq!(mapped, None);
}

#[test]
fn resolved_models_keep_the_list_order() {
    let world = World::new(&[
        plain("gpt-4", "dep-a", "openai/gpt-4"),
        plain("gpt-4", "dep-b", "azure/gpt-4"),
    ]);
    let models = world
        .queries()
        .resolved_litellm_models("gpt-4", None)
        .unwrap();
    assert_eq!(models, ["openai/gpt-4", "azure/gpt-4"]);
}

#[rstest]
#[case::an_empty_model(text(""))]
#[case::a_number_model(number(7))]
fn resolved_models_skip_a_model_that_is_not_a_name(#[case] provider_model: Json) {
    let world = World::new(&[deployment(
        "gpt-4",
        object(vec![("model", provider_model)]),
        object(vec![("id", text("dep-a"))]),
    )]);
    let models = world
        .queries()
        .resolved_litellm_models("gpt-4", None)
        .unwrap();
    assert!(models.is_empty());
}

#[test]
fn resolved_models_answer_nothing_for_an_unknown_name() {
    let world = World::new(&[plain("gpt-4", "dep-a", "openai/gpt-4")]);
    let models = world
        .queries()
        .resolved_litellm_models("unknown", None)
        .unwrap();
    assert!(models.is_empty());
}

#[test]
fn access_group_wildcard_route_is_found() {
    let world = World::new(&[
        grouped("team-alpha", "dep-a", &["beta"]),
        plain("team-*", "dep-b", "openai/m"),
    ]);
    assert!(world.queries().is_wildcard_access_group("beta").unwrap());
}

#[test]
fn access_group_plain_models_are_not_wildcards() {
    let world = World::new(&[
        grouped("gpt-4o", "dep-a", &["beta"]),
        plain("team-*", "dep-b", "openai/m"),
    ]);
    assert!(!world.queries().is_wildcard_access_group("beta").unwrap());
}

#[test]
fn access_group_unknown_group_is_not_a_wildcard() {
    let world = World::new(&[grouped("team-alpha", "dep-a", &["beta"])]);
    assert!(!world.queries().is_wildcard_access_group("nope").unwrap());
}

#[test]
fn order_levels_prefer_the_pre_routing_selection() {
    let world = World::new(&[
        params_ordered("gpt-4", "dep-a", 1),
        params_ordered("gpt-4", "dep-b", 2),
        params_ordered("tier", "dep-c", 7),
    ]);
    let call = Call::default().selecting(Some("tier"), None);
    let levels = world
        .queries()
        .messages_order_levels("gpt-4", &call.request())
        .unwrap();
    assert_eq!(levels, [7]);
}

#[test]
fn order_levels_skip_deployments_without_an_order() {
    let world = World::new(&[
        params_ordered("gpt-4", "dep-a", 1),
        plain("gpt-4", "dep-b", "openai/m"),
    ]);
    let call = Call::default();
    let levels = world
        .queries()
        .messages_order_levels("gpt-4", &call.request())
        .unwrap();
    assert_eq!(levels, [1]);
}

#[test]
fn order_levels_skip_a_non_integer_order() {
    let world = World::new(&[
        deployment(
            "gpt-4",
            object(vec![("model", text("openai/m")), ("order", text("first"))]),
            object(vec![("id", text("dep-a"))]),
        ),
        deployment(
            "gpt-4",
            object(vec![("model", text("openai/m"))]),
            object(vec![("id", text("dep-b")), ("order", text("second"))]),
        ),
        params_ordered("gpt-4", "dep-c", 2),
    ]);
    let call = Call::default();
    let levels = world
        .queries()
        .messages_order_levels("gpt-4", &call.request())
        .unwrap();
    assert_eq!(levels, [2]);
}

#[test]
fn order_levels_are_sorted_and_distinct() {
    let world = World::new(&[
        params_ordered("gpt-4", "dep-a", 3),
        info_ordered("gpt-4", "dep-b", 1),
        params_ordered("gpt-4", "dep-c", 3),
    ]);
    let call = Call::default();
    let levels = world
        .queries()
        .messages_order_levels("gpt-4", &call.request())
        .unwrap();
    assert_eq!(levels, [1, 3]);
}

#[test]
fn order_levels_read_the_metadata_team_id() {
    let world = World::new(&[
        params_ordered("shared", "dep-a", 1),
        team_ordered("internal-b", "dep-b", "team-1", "shared", 5),
    ]);
    let call = Call::default();
    let global = world
        .queries()
        .messages_order_levels("shared", &call.request())
        .unwrap();
    assert_eq!(global, [1]);
    let team_call = call.team("team-1");
    let owned = world
        .queries()
        .messages_order_levels("shared", &team_call.request())
        .unwrap();
    assert_eq!(owned, [5]);
}

#[test]
fn can_fall_back_is_false_when_fallbacks_are_disabled() {
    let world = World::new(&[
        params_ordered("gpt-4", "dep-a", 1),
        params_ordered("gpt-4", "dep-b", 2),
    ]);
    let call = Call::default().off();
    let config = RouterConfig::new(plans(vec![group("*", vec![name("backup")])], Vec::new()))
        .weighting(true);
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[rstest]
#[case::enabled(true, true)]
#[case::off(false, false)]
fn can_fall_back_reads_the_weighted_failover_section(
    #[case] enabled: bool,
    #[case] expected: bool,
) {
    let world = World::new(&[]);
    let call = Call::default();
    let config = RouterConfig::new(plans(Vec::new(), Vec::new())).weighting(enabled);
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert_eq!(answer, expected);
}

#[test]
fn can_fall_back_is_true_for_a_higher_order_level() {
    let world = World::new(&[
        params_ordered("gpt-4", "dep-a", 1),
        params_ordered("gpt-4", "dep-b", 2),
    ]);
    let call = Call::default().target(1);
    let config = RouterConfig::new(plans(Vec::new(), Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(answer);
}

#[test]
fn can_fall_back_is_false_at_the_top_order_level() {
    let world = World::new(&[
        params_ordered("gpt-4", "dep-a", 1),
        params_ordered("gpt-4", "dep-b", 2),
    ]);
    let call = Call::default().target(2);
    let config = RouterConfig::new(plans(Vec::new(), Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[test]
fn can_fall_back_starts_the_order_skip_at_the_first_level() {
    let world = World::new(&[
        params_ordered("gpt-4", "dep-a", 1),
        params_ordered("gpt-4", "dep-b", 2),
    ]);
    let call = Call::default();
    let config = RouterConfig::new(plans(Vec::new(), Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(answer);
}

#[test]
fn can_fall_back_ignores_a_single_order_level() {
    let world = World::new(&[params_ordered("gpt-4", "dep-a", 1)]);
    let call = Call::default().target(0);
    let config = RouterConfig::new(plans(Vec::new(), Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[test]
fn can_fall_back_is_true_for_the_router_content_policy_chain() {
    let world = World::new(&[]);
    let call = Call::default();
    let config = RouterConfig::new(plans(
        Vec::new(),
        vec![group("gpt-4", vec![name("backup")])],
    ));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(answer);
}

#[test]
fn can_fall_back_is_true_for_the_request_content_policy_chain() {
    let world = World::new(&[]);
    let call = Call::default().content_policy(vec![group("gpt-4", vec![name("backup")])]);
    let config = RouterConfig::new(plans(Vec::new(), Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(answer);
}

#[test]
fn can_fall_back_prefers_the_request_content_policy_list() {
    let world = World::new(&[]);
    let call = Call::default().content_policy(Vec::new());
    let config = RouterConfig::new(plans(
        Vec::new(),
        vec![group("gpt-4", vec![name("backup")])],
    ));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[test]
fn can_fall_back_skips_an_attempted_content_policy_target() {
    let world = World::new(&[]);
    let call = Call::default().tried(&[name("backup")]);
    let config = RouterConfig::new(plans(
        Vec::new(),
        vec![group("gpt-4", vec![name("backup")])],
    ));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[test]
fn can_fall_back_is_false_without_fallbacks() {
    let world = World::new(&[]);
    let call = Call::default();
    let config = RouterConfig::new(plans(Vec::new(), Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[test]
fn can_fall_back_hides_the_router_fallbacks_behind_an_empty_request_list() {
    let world = World::new(&[]);
    let call = Call::default().fallbacks(Vec::new());
    let config = RouterConfig::new(plans(
        vec![group("gpt-4", vec![name("backup")])],
        Vec::new(),
    ));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[test]
fn can_fall_back_reads_the_request_fallbacks_before_the_router_list() {
    let world = World::new(&[]);
    let call = Call::default()
        .fallbacks(vec![group("gpt-4", vec![name("request-target")])])
        .tried(&[name("request-target")]);
    let config = RouterConfig::new(plans(
        vec![group("gpt-4", vec![name("router-target")])],
        Vec::new(),
    ));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[test]
fn can_fall_back_is_true_for_a_client_fallback_list() {
    let world = World::new(&[]);
    let call = Call::default().fallbacks(vec![FallbackChainEntry::Target(name("claude"))]);
    let config = RouterConfig::new(plans(Vec::new(), Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(answer);
}

#[test]
fn can_fall_back_is_true_for_a_parameterized_client_target() {
    let world = World::new(&[]);
    let call = Call::default().fallbacks(vec![FallbackChainEntry::Target(
        FallbackTarget::Parameters(ParameterizedTarget {
            model: "claude".to_owned(),
            parameters: IndexMap::new(),
        }),
    )]);
    let config = RouterConfig::new(plans(Vec::new(), Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(answer);
}

#[test]
fn can_fall_back_is_false_when_every_target_was_attempted() {
    let world = World::new(&[]);
    let call = Call::default().tried(&[name("one"), name("two")]);
    let config = RouterConfig::new(plans(
        vec![group("gpt-4", vec![name("one"), name("two")])],
        Vec::new(),
    ));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(!answer);
}

#[test]
fn can_fall_back_reaches_the_generic_star_chain() {
    let world = World::new(&[]);
    let call = Call::default();
    let config = RouterConfig::new(plans(vec![group("*", vec![name("backup")])], Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(answer);
}

#[test]
fn can_fall_back_reads_the_request_model_group() {
    let world = World::new(&[]);
    let call = Call::default().selecting(None, Some("tier"));
    let config = RouterConfig::new(plans(vec![group("tier", vec![name("backup")])], Vec::new()));
    let answer = world
        .queries()
        .messages_can_fall_back("gpt-4", &call.request(), &config.fallbacks())
        .unwrap();
    assert!(answer);
}

// A deployment-info source that counts its reads.
struct InfoSource {
    answer: Option<JsonObject>,
    calls: Arc<AtomicU64>,
}

impl DeploymentInfo for InfoSource {
    type Error = Infallible;

    fn info(
        &self,
        _model_id: &str,
        _model_name: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Infallible>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = self.answer.clone();
        async move { Ok(answer) }
    }
}

struct InfoProvider {
    answers: BTreeMap<String, (String, String)>,
}

impl ProviderResolver for InfoProvider {
    type Error = Infallible;

    fn resolve_provider(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderAnswer, ProviderFailure<Infallible>> {
        match self.answers.get(request.model) {
            Some((litellm_model, provider)) => Ok(ProviderAnswer {
                litellm_model: litellm_model.clone(),
                provider: provider.clone(),
            }),
            None => Err(ProviderFailure::BadRequest(request.model.to_owned())),
        }
    }
}

struct UnitParams;

impl SupportedParameterSource for UnitParams {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

fn block_on<ReadFuture>(future: ReadFuture) -> ReadFuture::Output
where
    ReadFuture: Future,
{
    Builder::new_current_thread()
        .build()
        .expect("runtime")
        .block_on(future)
}

#[test]
fn cached_model_group_info_answers_from_the_cache() {
    let world = World::new(&[plain("g", "dep-a", "openai/m")]);
    let info = InfoSource {
        answer: Some(object(vec![("mode", text("chat"))])),
        calls: Arc::new(AtomicU64::new(0)),
    };
    let providers = InfoProvider {
        answers: BTreeMap::from([("openai/m".to_owned(), ("m".to_owned(), "openai".to_owned()))]),
    };
    let parameters = UnitParams;
    let cost_map = ModelCostMap::parse(b"{}").expect("the empty cost map");
    let fast_mode = RawCatalog::default();
    let sources = GroupSources {
        deployment_info: &info,
        providers: &providers,
        parameters: &parameters,
        cost_map: &cost_map,
        fast_mode_catalog: &fast_mode,
    };
    let queries = world.queries();
    let mut keys = ModelCostKeys::default();
    let first = block_on(queries.cached_model_group_info(&sources, "g", &mut keys))
        .expect("the first read")
        .expect("a group");
    let second = block_on(queries.cached_model_group_info(&sources, "g", &mut keys))
        .expect("the cached read")
        .expect("a group");
    assert_eq!(second, first);
    assert_eq!(info.calls.load(Ordering::SeqCst), 1);
}
