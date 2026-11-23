#![cfg(test)]

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};

use betterlite_context::anthropic_catalog_lookup::RawCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::names::ModelCostMap;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use tokio::runtime::Builder;

use crate::catalog_updates::catalog::DeploymentCatalog;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use crate::model_catalog_lookup::cache::{CacheClear, CacheSlot, CatalogCaches, Invalidation};
use crate::model_catalog_lookup::lookup::CatalogLookup;
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::ModelGroupInfo;
use crate::model_group_info::sources::{
    DeploymentInfo, GroupSources, ProviderAnswer, ProviderFailure, ProviderRequest,
    ProviderResolver,
};
use crate::model_group_lookups::access_groups::{AccessGroupMap, AccessGroups};
use crate::model_group_lookups::error::AccessGroupError;
use crate::model_group_lookups::group_info::GroupInfoCache;
use crate::routing_groups::index::RoutingGroupIndex;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn generated_id() -> String {
    "generated".to_owned()
}

fn plain_row(name: &str, id: &str) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", text("openai/m"))]),
        object(vec![("id", text(id))]),
        JsonObject::new(),
        generated_id,
    )
    .expect("the fixture deployment is invalid")
}

fn group_row(name: &str, id: &str, groups: &[&str]) -> DeploymentSpec {
    let listed: Vec<Json> = groups.iter().map(|group| text(group)).collect();
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", text("openai/m"))]),
        object(vec![
            ("id", text(id)),
            ("access_groups", Json::List(listed)),
        ]),
        JsonObject::new(),
        generated_id,
    )
    .expect("the fixture deployment is invalid")
}

fn team_row(name: &str, id: &str, team: &str, public: &str) -> DeploymentSpec {
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", text("openai/m"))]),
        object(vec![
            ("id", text(id)),
            ("team_id", text(team)),
            ("team_public_model_name", text(public)),
            ("access_groups", Json::List(vec![text("beta")])),
        ]),
        JsonObject::new(),
        generated_id,
    )
    .expect("the fixture deployment is invalid")
}

fn raw_row(name: &str, info: &str) -> DeploymentSpec {
    DeploymentSpec {
        model_name: name.to_owned(),
        litellm_params: object(vec![("model", text("openai/m"))]),
        model_info: serde_json::from_str(info).expect("model_info"),
        extra: JsonObject::new(),
    }
}

fn expected(entries: &[(&str, &[&str])]) -> AccessGroupMap {
    let mut map = AccessGroupMap::new();
    for (group, names) in entries {
        let listed: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
        map.insert((*group).to_owned(), listed);
    }
    map
}

const fn no_inferred(_model: &str) -> Option<String> {
    None
}

struct World {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
}

impl World {
    fn new(rows: &[DeploymentSpec]) -> Self {
        let mut world = Self {
            catalog: DeploymentCatalog::default(),
            aliases: AliasMap::new(),
            groups: RoutingGroupIndex::default(),
        };
        for row in rows {
            world.push(row.clone());
        }
        world
    }

    fn push(&mut self, row: DeploymentSpec) {
        self.catalog.index_patterns(&row).expect("pattern index");
        self.catalog.push(row);
    }

    fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: &no_inferred,
        }
    }
}

fn all_groups(world: &World, cache: &AccessGroups) -> Result<AccessGroupMap, AccessGroupError> {
    let lookup = world.lookup();
    cache.get_model_access_groups(&lookup, None, None, None)
}

#[test]
fn access_groups_appends_once_per_occurrence() {
    let world = World::new(&[
        group_row("gpt-4o", "a", &["beta", "beta"]),
        group_row("claude", "b", &["beta", "alpha"]),
        group_row("claude", "c", &["beta"]),
    ]);
    let cache = AccessGroups::default();
    let map = all_groups(&world, &cache).expect("the map");
    assert_eq!(
        map,
        expected(&[
            ("beta", &["gpt-4o", "gpt-4o", "claude", "claude"]),
            ("alpha", &["claude"]),
        ])
    );
}

#[test]
fn access_groups_lists_alias_rows_before_deployment_rows() {
    let mut world = World::new(&[group_row("gpt-4o", "a", &["beta"])]);
    world
        .aliases
        .insert("sonnet".to_owned(), AliasTarget::Name("gpt-4o".to_owned()));
    let cache = AccessGroups::default();
    let map = all_groups(&world, &cache).expect("the map");
    assert_eq!(map, expected(&[("beta", &["sonnet", "gpt-4o"])]));
}

#[rstest]
#[case::exact("beta", &["gpt-4o"])]
#[case::partial("bet", &[])]
#[case::suffix("beta-2", &["claude"])]
fn access_groups_keeps_only_the_exact_group(#[case] filter: &str, #[case] names: &[&str]) {
    let world = World::new(&[
        group_row("gpt-4o", "a", &["beta"]),
        group_row("claude", "b", &["beta-2"]),
    ]);
    let cache = AccessGroups::default();
    let lookup = world.lookup();
    let map = cache
        .get_model_access_groups(&lookup, None, Some(filter), None)
        .expect("the map");
    let wanted = if names.is_empty() {
        AccessGroupMap::new()
    } else {
        expected(&[(filter, names)])
    };
    assert_eq!(map, wanted);
}

#[test]
fn access_groups_narrows_by_model_name() {
    let world = World::new(&[
        group_row("gpt-4o", "a", &["beta"]),
        group_row("claude", "b", &["beta"]),
    ]);
    let cache = AccessGroups::default();
    let lookup = world.lookup();
    let map = cache
        .get_model_access_groups(&lookup, Some("gpt-4o"), None, None)
        .expect("the map");
    assert_eq!(map, expected(&[("beta", &["gpt-4o"])]));
}

#[test]
fn access_groups_narrows_by_team() {
    let world = World::new(&[
        team_row("team-shared", "a", "team-1", "team-model"),
        team_row("team-other", "b", "team-2", "team-model"),
        group_row("gpt-4o", "c", &["beta"]),
    ]);
    let cache = AccessGroups::default();
    let lookup = world.lookup();
    let narrowed = cache
        .get_model_access_groups(&lookup, Some("team-model"), None, Some("team-1"))
        .expect("the team map");
    assert_eq!(narrowed, expected(&[("beta", &["team-shared"])]));
    let unowned = cache
        .get_model_access_groups(&lookup, Some("team-model"), None, None)
        .expect("the unowned map");
    assert!(unowned.is_empty());
}

#[test]
fn access_groups_names_wildcard_rows_after_the_request() {
    let world = World::new(&[group_row("anthropic/*", "a", &["beta"])]);
    let cache = AccessGroups::default();
    let lookup = world.lookup();
    let map = cache
        .get_model_access_groups(&lookup, Some("anthropic/claude-3"), None, None)
        .expect("the map");
    assert_eq!(map, expected(&[("beta", &["anthropic/claude-3"])]));
}

#[test]
fn access_groups_skips_empty_info_and_null_groups() {
    let empty = DeploymentSpec {
        model_name: "empty".to_owned(),
        litellm_params: object(vec![("model", text("openai/m"))]),
        model_info: JsonObject::new(),
        extra: JsonObject::new(),
    };
    let world = World::new(&[
        empty,
        raw_row("null-groups", r#"{"access_groups":null}"#),
        plain_row("plain", "c"),
        group_row("gpt-4o", "a", &["beta"]),
    ]);
    let cache = AccessGroups::default();
    let map = all_groups(&world, &cache).expect("the map");
    assert_eq!(map, expected(&[("beta", &["gpt-4o"])]));
}

#[rstest]
#[case::text(r#"{"access_groups":"beta"}"#)]
#[case::object(r#"{"access_groups":{"beta":true}}"#)]
#[case::entry(r#"{"access_groups":["beta",2]}"#)]
fn access_groups_rejects_a_value_outside_the_list(#[case] info: &str) {
    let world = World::new(&[raw_row("bad-row", info)]);
    let cache = AccessGroups::default();
    let error =
        all_groups(&world, &cache).expect_err("the access_groups value is not a list of strings");
    assert!(
        matches!(error, AccessGroupError::InvalidAccessGroups { ref model_name } if model_name == "bad-row")
    );
}

#[test]
fn access_groups_caches_only_the_no_argument_call() {
    let mut world = World::new(&[group_row("gpt-4o", "a", &["beta"])]);
    let cache = AccessGroups::default();
    let first = all_groups(&world, &cache).expect("the first map");
    world.push(group_row("claude", "b", &["beta"]));
    let cached = all_groups(&world, &cache).expect("the cached map");
    assert_eq!(cached, first);
    cache.clear();
    let followed = all_groups(&world, &cache).expect("the followed map");
    assert_eq!(followed, expected(&[("beta", &["gpt-4o", "claude"])]));
}

#[test]
fn access_groups_caches_an_empty_map() {
    let mut world = World::new(&[plain_row("gpt-4o", "a")]);
    let cache = AccessGroups::default();
    let mut first = all_groups(&world, &cache).expect("the empty map");
    assert!(first.is_empty());
    // the answer is the caller's own map, so changing it cannot reach the cached slot
    first.insert("gamma".to_owned(), vec!["gpt-4o".to_owned()]);
    world.push(group_row("claude", "b", &["beta"]));
    assert!(
        all_groups(&world, &cache)
            .expect("the cached map")
            .is_empty()
    );
    cache.clear();
    let followed = all_groups(&world, &cache).expect("the followed map");
    assert_eq!(followed, expected(&[("beta", &["claude"])]));
}

#[rstest]
#[case::model_name(Some(""), None, None)]
#[case::model_access_group(None, Some(""), None)]
#[case::team_id(None, None, Some(""))]
fn access_groups_empty_string_argument_skips_the_cache(
    #[case] model_name: Option<&str>,
    #[case] model_access_group: Option<&str>,
    #[case] team_id: Option<&str>,
) {
    let mut world = World::new(&[group_row("gpt-4o", "a", &["beta"])]);
    let cache = AccessGroups::default();
    {
        let lookup = world.lookup();
        let _ignored = cache
            .get_model_access_groups(&lookup, model_name, model_access_group, team_id)
            .expect("the empty-string map");
    }
    world.push(group_row("claude", "b", &["beta"]));
    let map = all_groups(&world, &cache).expect("the map");
    assert_eq!(map, expected(&[("beta", &["gpt-4o", "claude"])]));
}

#[test]
fn access_groups_lookup_error_leaves_the_cache_empty() {
    let mut world = World::new(&[group_row("gpt-4o", "a", &["beta"])]);
    world.aliases.insert(
        "broken".to_owned(),
        AliasTarget::Item {
            model: Some(text("gpt-4o")),
            hidden: None,
        },
    );
    let cache = AccessGroups::default();
    {
        let lookup = world.lookup();
        let error = cache
            .get_model_access_groups(&lookup, None, None, None)
            .expect_err("the alias has no hidden field");
        assert!(matches!(error, AccessGroupError::Lookup(_)));
    }
    world
        .aliases
        .insert("broken".to_owned(), AliasTarget::Name("gpt-4o".to_owned()));
    let map = all_groups(&world, &cache).expect("the map");
    assert_eq!(map, expected(&[("beta", &["broken", "gpt-4o"])]));
}

#[test]
fn access_groups_invalidation_clears_only_access_groups() {
    let mut world = World::new(&[group_row("gpt-4o", "a", &["beta"])]);
    let cache = AccessGroups::default();
    let metadata = CacheSlot::<String>::default();
    let deployment = CacheSlot::<String>::default();
    let zero_cost = CacheSlot::<bool>::default();
    let rows = CacheSlot::<String>::default();
    *metadata.0.lock().unwrap() = Some("stale".to_owned());
    let caches = CatalogCaches {
        model_group_info: &metadata,
        deployment_model_info: &deployment,
        zero_cost: &zero_cost,
        routing_group_rows: &rows,
        access_groups: &cache,
    };
    let first = all_groups(&world, &cache).expect("the first map");
    world.push(group_row("claude", "b", &["beta"]));
    caches.invalidate(Invalidation::ModelMetadata);
    assert_eq!(*metadata.0.lock().unwrap(), None);
    assert_eq!(all_groups(&world, &cache).expect("the cached map"), first);
    caches.invalidate(Invalidation::AccessGroups);
    let followed = all_groups(&world, &cache).expect("the followed map");
    assert_eq!(followed, expected(&[("beta", &["gpt-4o", "claude"])]));
}

// A metadata source that counts its reads, and one whose answer waits for release while its gate is set.
struct FakeInfo {
    answer: Option<JsonObject>,
    calls: Arc<AtomicU64>,
    gate: Option<Arc<AtomicBool>>,
}

impl DeploymentInfo for FakeInfo {
    type Error = Infallible;

    fn info(
        &self,
        _model_id: &str,
        _model_name: &str,
        _keys: &mut ModelCostKeys,
    ) -> impl Future<Output = Result<Option<JsonObject>, Infallible>> + Send {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = self.answer.clone();
        let gate = self.gate.clone();
        async move {
            if let Some(gate) = gate {
                std::future::poll_fn(move |_context| {
                    if gate.load(Ordering::SeqCst) {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
            Ok(answer)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("the test asked for a provider error")]
struct ProviderUnavailable;

struct FakeProvider {
    answers: BTreeMap<String, (String, String)>,
    failing: Arc<AtomicBool>,
}

impl FakeProvider {
    fn new(failing: &Arc<AtomicBool>) -> Self {
        Self {
            answers: BTreeMap::from([(
                "openai/m".to_owned(),
                ("m".to_owned(), "openai".to_owned()),
            )]),
            failing: Arc::clone(failing),
        }
    }
}

impl ProviderResolver for FakeProvider {
    type Error = ProviderUnavailable;

    fn resolve_provider(
        &self,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderAnswer, ProviderFailure<ProviderUnavailable>> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(ProviderFailure::Other(ProviderUnavailable));
        }
        match self.answers.get(request.model) {
            Some((litellm_model, provider)) => Ok(ProviderAnswer {
                litellm_model: litellm_model.clone(),
                provider: provider.clone(),
            }),
            None => Err(ProviderFailure::BadRequest(request.model.to_owned())),
        }
    }
}

struct FakeParams;

impl SupportedParameterSource for FakeParams {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

fn cost_map() -> ModelCostMap {
    ModelCostMap::parse(b"{}").expect("the empty cost map")
}

struct Scenario {
    world: World,
    info: FakeInfo,
    providers: FakeProvider,
    parameters: FakeParams,
    cost_map: ModelCostMap,
    fast_mode: RawCatalog,
    cache: GroupInfoCache,
    failing: Arc<AtomicBool>,
}

impl Scenario {
    fn new(capacity: i64, rows: &[DeploymentSpec]) -> Self {
        Self::build(capacity, rows, None)
    }

    fn gated(rows: &[DeploymentSpec]) -> Self {
        Self::build(64, rows, Some(Arc::new(AtomicBool::new(false))))
    }

    fn build(capacity: i64, rows: &[DeploymentSpec], gate: Option<Arc<AtomicBool>>) -> Self {
        let failing = Arc::new(AtomicBool::new(false));
        Self {
            world: World::new(rows),
            info: FakeInfo {
                answer: Some(object(vec![("mode", text("chat"))])),
                calls: Arc::new(AtomicU64::new(0)),
                gate,
            },
            providers: FakeProvider::new(&failing),
            parameters: FakeParams,
            cost_map: cost_map(),
            fast_mode: RawCatalog::default(),
            cache: GroupInfoCache::new(capacity),
            failing,
        }
    }

    fn sources(&self) -> GroupSources<'_, FakeInfo, FakeProvider, FakeParams> {
        GroupSources {
            deployment_info: &self.info,
            providers: &self.providers,
            parameters: &self.parameters,
            cost_map: &self.cost_map,
            fast_mode_catalog: &self.fast_mode,
        }
    }

    fn read(
        &self,
        model_group: &str,
    ) -> Result<Option<ModelGroupInfo>, GroupInfoError<ProviderUnavailable>> {
        let lookup = self.world.lookup();
        let sources = self.sources();
        let mut keys = ModelCostKeys::default();
        block_on(
            self.cache
                .cached_get_model_group_info(&lookup, &sources, model_group, &mut keys),
        )
    }

    fn release(&self) {
        if let Some(gate) = &self.info.gate {
            gate.store(true, Ordering::SeqCst);
        }
    }

    fn calls(&self) -> u64 {
        self.info.calls.load(Ordering::SeqCst)
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
fn group_info_hit_skips_the_read() {
    let scenario = Scenario::new(64, &[plain_row("g", "a")]);
    let first = scenario.read("g").expect("the first read").expect("group");
    assert_eq!(scenario.calls(), 1);
    let second = scenario.read("g").expect("the second read").expect("group");
    assert_eq!(scenario.calls(), 1);
    assert_eq!(second, first);
}

#[test]
fn group_info_caches_a_none_answer() {
    let mut scenario = Scenario::new(64, &[]);
    assert_eq!(scenario.read("g").expect("the first read"), None);
    scenario.world.push(plain_row("g", "a"));
    assert_eq!(scenario.read("g").expect("the cached read"), None);
    scenario.cache.clear();
    assert!(scenario.read("g").expect("the followed read").is_some());
}

#[test]
fn group_info_does_not_store_an_error() {
    let scenario = Scenario::new(64, &[plain_row("g", "a")]);
    scenario.failing.store(true, Ordering::SeqCst);
    let error = scenario.read("g").expect_err("the provider fails");
    assert!(matches!(error, GroupInfoError::Provider { .. }));
    scenario.failing.store(false, Ordering::SeqCst);
    assert!(scenario.read("g").expect("the second read").is_some());
}

#[test]
fn group_info_evicts_at_capacity_one() {
    let scenario = Scenario::new(1, &[plain_row("a", "a"), plain_row("b", "b")]);
    assert!(scenario.read("a").expect("a").is_some());
    assert!(scenario.read("b").expect("b").is_some());
    assert_eq!(scenario.calls(), 2);
    assert!(scenario.read("a").expect("a again").is_some());
    assert_eq!(scenario.calls(), 3);
}

#[test]
fn group_info_evicts_the_least_recent() {
    let scenario = Scenario::new(
        2,
        &[
            plain_row("a", "a"),
            plain_row("b", "b"),
            plain_row("c", "c"),
        ],
    );
    assert!(scenario.read("a").expect("a").is_some());
    assert!(scenario.read("b").expect("b").is_some());
    assert!(scenario.read("a").expect("a again").is_some());
    assert_eq!(scenario.calls(), 2);
    assert!(scenario.read("c").expect("c").is_some());
    assert_eq!(scenario.calls(), 3);
    assert!(scenario.read("a").expect("a kept").is_some());
    assert_eq!(scenario.calls(), 3);
    assert!(scenario.read("b").expect("b back").is_some());
    assert_eq!(scenario.calls(), 4);
}

#[rstest]
#[case::zero(0)]
#[case::negative(-1)]
fn group_info_stores_nothing_at_a_non_positive_capacity(#[case] capacity: i64) {
    let scenario = Scenario::new(capacity, &[plain_row("g", "a")]);
    assert!(scenario.read("g").expect("the first read").is_some());
    assert!(scenario.read("g").expect("the second read").is_some());
    assert_eq!(scenario.calls(), 2);
}

#[test]
fn group_info_invalidation_clears_only_model_metadata() {
    let scenario = Scenario::new(64, &[plain_row("g", "a")]);
    let access = CacheSlot::<AccessGroupMap>::default();
    let deployment = CacheSlot::<String>::default();
    let zero_cost = CacheSlot::<bool>::default();
    let rows = CacheSlot::<String>::default();
    let caches = CatalogCaches {
        model_group_info: &scenario.cache,
        deployment_model_info: &deployment,
        zero_cost: &zero_cost,
        routing_group_rows: &rows,
        access_groups: &access,
    };
    assert!(scenario.read("g").expect("the first read").is_some());
    caches.invalidate(Invalidation::ModelMetadata);
    assert!(scenario.read("g").expect("the second read").is_some());
    assert_eq!(scenario.calls(), 2);
    assert!(scenario.read("g").expect("the third read").is_some());
    assert_eq!(scenario.calls(), 2);
    caches.invalidate(Invalidation::AccessGroups);
    assert!(scenario.read("g").expect("the fourth read").is_some());
    assert_eq!(scenario.calls(), 2);
}

#[test]
fn group_info_keeps_each_cache_instance_apart() {
    let first = Scenario::new(64, &[plain_row("g", "a")]);
    let second = Scenario::new(64, &[plain_row("g", "a")]);
    assert!(first.read("g").expect("the first read").is_some());
    assert!(second.read("g").expect("the other read").is_some());
    first.cache.clear();
    assert!(first.read("g").expect("the first recompute").is_some());
    assert_eq!(first.calls(), 2);
    assert!(second.read("g").expect("the other hit").is_some());
    assert_eq!(second.calls(), 1);
}

#[tokio::test]
async fn group_info_a_clear_during_a_read_keeps_the_older_answer_out() {
    let scenario = Scenario::gated(&[plain_row("g", "a")]);
    let lookup = scenario.world.lookup();
    let sources = scenario.sources();
    let cache = &scenario.cache;
    {
        let mut keys = ModelCostKeys::default();
        let mut read =
            std::pin::pin!(cache.cached_get_model_group_info(&lookup, &sources, "g", &mut keys));
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(read.as_mut().poll(&mut context), Poll::Pending));
        cache.clear();
        scenario.release();
        let older = read.await.expect("the read").expect("group");
        assert_eq!(older.model_group, "g");
    }
    assert_eq!(scenario.calls(), 1);
    let mut keys = ModelCostKeys::default();
    let newer = cache
        .cached_get_model_group_info(&lookup, &sources, "g", &mut keys)
        .await
        .expect("the second read")
        .expect("group");
    assert_eq!(newer.model_group, "g");
    assert_eq!(scenario.calls(), 2);
}
