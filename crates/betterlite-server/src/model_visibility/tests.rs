#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_config::sections::model_visibility::Settings;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_router::catalog_updates::catalog::DeploymentCatalog;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::{AliasMap, AliasTarget};
use betterlite_router::model_catalog_lookup::lookup::CatalogLookup;
use betterlite_router::routing_groups::index::RoutingGroupIndex;
use betterlite_router::wildcard_catalog::Candidate;
use indexmap::IndexMap;
use serde_json::Number;
use tokio::runtime::Builder;
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use crate::model_visibility::discoverable::{discoverable_rows, undiscoverable_model_names};
use crate::model_visibility::healthy::{RouterHealth, hidden_unhealthy_model_names};

type Provider = Box<dyn Fn(&str) -> Option<String>>;

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn object(entries: Vec<(&str, Json)>) -> JsonObject {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn row(name: &str, id: &str, info: Vec<(&str, Json)>) -> DeploymentSpec {
    let mut model_info = object(info);
    if !model_info.contains_key("id") {
        model_info.insert("id".to_owned(), text(id));
    }
    DeploymentSpec::new(
        name.to_owned(),
        object(vec![("model", text("openai/gpt-4o"))]),
        model_info,
        JsonObject::new(),
        String::new,
    )
    .unwrap()
}

fn identified(name: &str, id: &str) -> DeploymentSpec {
    row(name, id, Vec::new())
}

fn hidden(name: &str, id: &str) -> DeploymentSpec {
    row(name, id, vec![("discoverable", Json::Bool(false))])
}

struct Fixture {
    catalog: DeploymentCatalog,
    aliases: AliasMap,
    groups: RoutingGroupIndex,
    provider: Provider,
}

impl Fixture {
    fn new(rows: &[DeploymentSpec]) -> Self {
        let mut catalog = DeploymentCatalog::default();
        for spec in rows {
            catalog.model_names.insert(spec.model_name.clone());
            catalog.index_patterns(spec).unwrap();
            catalog.push(spec.clone());
        }
        Self {
            catalog,
            aliases: IndexMap::new(),
            groups: RoutingGroupIndex::default(),
            provider: Box::new(|_model: &str| None),
        }
    }

    fn with_alias(mut self, alias: &str, target: AliasTarget) -> Self {
        self.aliases.insert(alias.to_owned(), target);
        self
    }

    fn with_provider(mut self, resolved: Vec<(&str, &str)>) -> Self {
        let by_name: BTreeMap<String, String> = resolved
            .into_iter()
            .map(|(model, provider)| (model.to_owned(), provider.to_owned()))
            .collect();
        self.provider = Box::new(move |model: &str| by_name.get(model).cloned());
        self
    }

    // the production wiring answers get_llm_provider, whose None the source's f-string turns into a "None/" prefix
    fn with_unresolved_provider(mut self) -> Self {
        self.provider = Box::new(|_model: &str| Some("None".to_owned()));
        self
    }

    // a pattern whose registration no deployment followed, which a lookup of its names fails on
    fn with_broken_pattern(mut self, pattern: &str) -> Self {
        self.catalog
            .pattern_router
            .add_pattern(
                pattern,
                Candidate {
                    model_id: None,
                    model: "openai/gpt-4o".to_owned(),
                },
            )
            .unwrap();
        self
    }

    fn lookup(&self) -> CatalogLookup<'_> {
        CatalogLookup {
            catalog: &self.catalog,
            model_group_alias: &self.aliases,
            routing_groups: &self.groups,
            inferred_provider: self.provider.as_ref(),
        }
    }
}

fn user(role: Option<&str>) -> UserAPIKeyAuth {
    let mut fields: BTreeMap<String, JsonData> = BTreeMap::new();
    fields.insert("api_key".to_owned(), JsonData::String("sk-test".to_owned()));
    if let Some(assigned) = role {
        fields.insert(
            "user_role".to_owned(),
            JsonData::String(assigned.to_owned()),
        );
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn hidden_names(
    fixture: &Fixture,
    asked: &[&str],
    role: Option<&str>,
    team: Option<&str>,
) -> Vec<String> {
    let lookup = fixture.lookup();
    undiscoverable_model_names(&names(asked), Some(&lookup), &user(role), team)
        .unwrap()
        .into_iter()
        .collect()
}

#[test]
fn undiscoverable_model() {
    let fixture = Fixture::new(&[
        hidden("hidden", "a"),
        hidden("hidden", "b"),
        hidden("mixed", "c"),
        identified("mixed", "d"),
        identified("visible", "e"),
        row(
            "zero",
            "f",
            vec![("discoverable", Json::Number(Number::from(0_u8)))],
        ),
        row("quoted", "g", vec![("discoverable", text("false"))]),
    ]);
    let asked = ["hidden", "mixed", "visible", "zero", "quoted", "missing"];
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), None),
        ["hidden"]
    );
}

#[test]
fn admin_view_bypasses_the_filter() {
    let fixture = Fixture::new(&[hidden("hidden", "a"), identified("visible", "b")]);
    let asked = ["hidden", "visible"];
    assert!(hidden_names(&fixture, &asked, Some("proxy_admin"), None).is_empty());
    assert!(hidden_names(&fixture, &asked, Some("proxy_admin_viewer"), None).is_empty());
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), None),
        ["hidden"]
    );
    assert_eq!(hidden_names(&fixture, &asked, None, None), ["hidden"]);
    let no_router =
        undiscoverable_model_names(&names(&asked), None, &user(Some("internal_user")), None);
    assert!(no_router.unwrap().is_empty());
}

#[test]
fn team_public_alias() {
    let fixture = Fixture::new(&[
        row(
            "team-group",
            "t1",
            vec![
                ("team_id", text("team-a")),
                ("team_public_model_name", text("team-public")),
                ("discoverable", Json::Bool(false)),
            ],
        ),
        hidden("global-hidden", "g1"),
        identified("visible", "v1"),
    ]);
    let asked = ["team-group", "team-public", "global-hidden", "visible"];
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), Some("team-a")),
        ["global-hidden", "team-group", "team-public"]
    );
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), Some("team-b")),
        ["global-hidden"]
    );
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), None),
        ["global-hidden", "team-group"]
    );
    assert_eq!(
        hidden_names(&fixture, &asked, Some("proxy_admin"), Some("team-b")),
        Vec::<String>::new()
    );
}

#[test]
fn alias_targets_hide_their_hidden_target() {
    let fixture = Fixture::new(&[hidden("global-hidden", "g1"), identified("visible", "v1")])
        .with_alias("alias-name", AliasTarget::Name("global-hidden".to_owned()))
        .with_alias(
            "alias-item",
            AliasTarget::Item {
                model: Some(text("global-hidden")),
                hidden: Some(Json::Bool(false)),
            },
        )
        .with_alias("alias-visible", AliasTarget::Name("visible".to_owned()));
    let asked = ["alias-name", "alias-item", "alias-visible", "visible"];
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), None),
        ["alias-item", "alias-name"]
    );
}

#[test]
fn wildcard_hidden() {
    let fixture = Fixture::new(&[
        hidden("gpt-*", "w1"),
        hidden("shared/*", "w2"),
        identified("shared/visible", "w3"),
        identified("open/*", "w4"),
    ]);
    let asked = ["gpt-4o", "shared/x", "open/x", "nothing"];
    // a pattern with any visible deployment is not a hidden pattern, so "open/x" stays listed
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), None),
        ["gpt-4o", "shared/x"]
    );
}

#[test]
fn pattern_matches_the_provider_prefixed_name() {
    let fixture =
        Fixture::new(&[hidden("openai/*", "w1")]).with_provider(vec![("gpt-4o", "openai")]);
    let asked = ["gpt-4o", "claude-3"];
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), None),
        ["gpt-4o"]
    );
}

#[test]
fn unresolved_provider_matches_the_literal_none_prefix() {
    let fixture = Fixture::new(&[hidden("None/*", "w1"), identified("wat/wat", "v1")])
        .with_unresolved_provider();
    assert_eq!(
        hidden_names(&fixture, &["wat/other"], Some("internal_user"), None),
        ["wat/other"]
    );
    // the provider letter is matched, but a visible sibling resolved for the name keeps it listed
    assert!(hidden_names(&fixture, &["wat/wat"], Some("internal_user"), None).is_empty());
}

#[test]
fn hidden_team_pattern_applies_only_to_its_team() {
    let fixture = Fixture::new(&[
        row(
            "team-group",
            "t1",
            vec![
                ("team_id", text("team-a")),
                ("team_public_model_name", text("team/*")),
                ("discoverable", Json::Bool(false)),
            ],
        ),
        hidden("team/x", "x1"),
    ]);
    let asked = ["team/x", "team/y", "team-group"];
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), Some("team-a")),
        ["team-group", "team/x", "team/y"]
    );
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), Some("team-b")),
        ["team/x"]
    );
    assert_eq!(
        hidden_names(&fixture, &asked, Some("internal_user"), None),
        ["team-group", "team/x"]
    );
}

#[test]
fn no_hidden_rows_stop_the_lookup() {
    let fixture = Fixture::new(&[identified("visible", "v1")]);
    assert!(
        hidden_names(
            &fixture,
            &["visible", "missing"],
            Some("internal_user"),
            None
        )
        .is_empty()
    );
}

#[test]
fn only_candidates_are_looked_up() {
    let fixture = Fixture::new(&[hidden("hidden", "h1")]).with_broken_pattern("broken-*");
    // the pattern is nobody's hidden row, so the filter never resolves its names and its broken row cannot fail the listing
    assert!(hidden_names(&fixture, &["broken-x"], Some("internal_user"), None).is_empty());
    assert_eq!(
        hidden_names(&fixture, &["hidden"], Some("internal_user"), None),
        ["hidden"]
    );
}

#[test]
fn discoverable_rows_keep_order_and_duplicates() {
    let rows = [
        identified("first", "a"),
        hidden("hidden", "b"),
        identified("first", "a"),
        identified("last", "c"),
    ];
    let kept = discoverable_rows(&rows, &user(Some("internal_user")));
    let kept_names: Vec<&str> = kept.iter().map(|spec| spec.model_name.as_str()).collect();
    assert_eq!(kept_names, ["first", "first", "last"]);
    assert_eq!(
        discoverable_rows(&rows, &user(Some("proxy_admin"))).len(),
        4
    );
}

struct HealthReport {
    names: Result<BTreeSet<String>, String>,
    calls: Arc<AtomicUsize>,
}

impl HealthReport {
    fn ready(values: &[&str]) -> Self {
        Self {
            names: Ok(values.iter().map(|value| (*value).to_owned()).collect()),
            calls: Arc::default(),
        }
    }

    fn failed() -> Self {
        Self {
            names: Err("the health cache is unreachable".to_owned()),
            calls: Arc::default(),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl RouterHealth for HealthReport {
    type Error = String;

    async fn fully_unhealthy_model_names(&self) -> Result<BTreeSet<String>, String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.names.clone()
    }
}

#[derive(Clone, Default)]
struct DebugCounter(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for DebugCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

const fn settings(enabled: bool) -> Settings {
    Settings {
        model_list_healthy_only: enabled,
    }
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[test]
fn healthy_only_default() {
    let report = HealthReport::ready(&["hidden"]);
    let hidden = block_on(hidden_unhealthy_model_names(
        None,
        settings(true),
        Some(&report),
    ))
    .unwrap();
    assert_eq!(hidden, BTreeSet::from(["hidden".to_owned()]));
    assert_eq!(report.calls(), 1);
    let off = HealthReport::ready(&["hidden"]);
    let skipped = block_on(hidden_unhealthy_model_names(
        None,
        settings(false),
        Some(&off),
    ))
    .unwrap();
    assert!(skipped.is_empty());
    assert_eq!(off.calls(), 0);
    let missing_router: BTreeSet<String> = block_on(hidden_unhealthy_model_names(
        None,
        settings(true),
        None::<&HealthReport>,
    ))
    .unwrap();
    assert!(missing_router.is_empty());
}

#[test]
fn explicit_healthy_only() {
    let requested = HealthReport::ready(&["hidden"]);
    let hidden = block_on(hidden_unhealthy_model_names(
        Some(true),
        settings(false),
        Some(&requested),
    ))
    .unwrap();
    assert_eq!(hidden, BTreeSet::from(["hidden".to_owned()]));
    assert_eq!(requested.calls(), 1);
    // a request cannot turn the proxy-wide opt-in back off
    let global = HealthReport::ready(&["hidden"]);
    let kept_by_setting = block_on(hidden_unhealthy_model_names(
        Some(false),
        settings(true),
        Some(&global),
    ))
    .unwrap();
    assert_eq!(kept_by_setting, BTreeSet::from(["hidden".to_owned()]));
    assert_eq!(global.calls(), 1);
    let refused = HealthReport::ready(&["hidden"]);
    let not_enabled = block_on(hidden_unhealthy_model_names(
        Some(false),
        settings(false),
        Some(&refused),
    ))
    .unwrap();
    assert!(not_enabled.is_empty());
    assert_eq!(refused.calls(), 0);
}

#[test]
fn unknown_health() {
    let report = HealthReport::ready(&[]);
    let counter = DebugCounter::default();
    let subscriber = tracing_subscriber::registry().with(counter.clone());
    let hidden = with_default(subscriber, || {
        block_on(hidden_unhealthy_model_names(
            None,
            settings(true),
            Some(&report),
        ))
    })
    .unwrap();
    assert!(hidden.is_empty());
    assert_eq!(counter.0.load(Ordering::Relaxed), 1);
    assert_eq!(report.calls(), 1);
}

#[test]
fn health_error_propagates() {
    let report = HealthReport::failed();
    let error = block_on(hidden_unhealthy_model_names(
        None,
        settings(true),
        Some(&report),
    ))
    .unwrap_err();
    assert_eq!(error, "the health cache is unreachable");
    assert_eq!(report.calls(), 1);
}
