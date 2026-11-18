use std::sync::Arc;
use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::support::{
    Counted, Fixture, LastOnly, NoServices, RequestSetup, body, clock, deployment, group_and_ids,
    hidden, in_flight_store, metadata, number, object, paused, picked, run_healthy, run_route,
    run_simple, seeded, text, weighted, with_info,
};
use crate::deployment_catalog::types::{AliasSource, AliasStep, Candidates};
use crate::deployment_clients::clients::{CachedClient, CachedClients, ClientCredential};
use crate::deployment_identity::spec::DeploymentSpec;
use crate::least_busy::identity::request_count_key;
use crate::least_busy::{IN_FLIGHT_COUNT_TTL_SECONDS, LeastBusy};
use crate::router_core::attempt::AttemptResult;
use crate::router_core::headers::{
    apply_response_model_id, ensure_additional_headers, prepare_response,
    response_has_hidden_params, response_total_token_count, write_hidden_params,
};
use crate::router_core::pipeline::HealthyDeployments;
use crate::routing_value_store::{RouteKey, RouteValue, StoreRead};
use crate::strategy::Strategy;
use crate::strategy_dispatch::strategy::RoutingStrategy;

pub(super) async fn weighted_pick() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 5), weighted("gpt-4o", "b", 0)]);
    let result = run_simple(&fixture, &RequestSetup::new("gpt-4o"))
        .await
        .unwrap();
    assert_eq!(result.group, "gpt-4o");
    assert_eq!(picked(&result), "a");
    assert_eq!(result.strategy, Some(Strategy::SimpleShuffle));
    assert!(result.aliases.is_empty());
    assert!(result.client.is_none());
}

pub(super) async fn shared_pool_chat_and_embedding() {
    let deployments = vec![
        weighted("shared", "chat", 1),
        deployment(
            "shared",
            "embed",
            &[
                ("model", text("openai/text-embedding-3-small")),
                ("weight", number(0)),
            ],
        ),
    ];
    let fixture = Fixture::plain(deployments);
    let chat = RequestSetup::new("shared");
    let mut embeddings = RequestSetup::new("shared");
    embeddings.excluded = vec![String::from("chat")];
    let chat_result = run_simple(&fixture, &chat).await.unwrap();
    let embed_result = run_simple(&fixture, &embeddings).await.unwrap();
    assert_eq!(chat_result.group, "shared");
    assert_eq!(picked(&chat_result), "chat");
    assert_eq!(embed_result.group, "shared");
    assert_eq!(picked(&embed_result), "embed");
    assert_eq!(
        embed_result.deployment.litellm_params.get("model"),
        Some(&text("openai/text-embedding-3-small"))
    );
}

pub(super) async fn callback_narrowed() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 5), weighted("gpt-4o", "b", 0)]);
    let mut random = seeded();
    let result = run_route(
        &fixture,
        &RequestSetup::new("gpt-4o"),
        &LastOnly,
        RoutingStrategy::SimpleShuffle,
        &mut random,
    )
    .await
    .unwrap();
    assert_eq!(picked(&result), "b");
    assert_eq!(result.strategy, Some(Strategy::SimpleShuffle));
}

pub(super) async fn specific_deployment_pool() {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let mut setup = RequestSetup::new("openai/a");
    setup.specific_deployment = true;
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(result.group, "openai/a");
    assert_eq!(picked(&result), "a");
    assert_eq!(result.strategy, Some(Strategy::SimpleShuffle));
    assert!(result.aliases.is_empty());
}

pub(super) async fn pinned_short_circuit() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]));
    let mut pinned = RequestSetup::new("gpt-4o");
    pinned.pinned = true;
    pinned.excluded = vec![String::from("b")];
    let mut bare = RequestSetup::new("gpt-4o");
    bare.excluded = vec![String::from("b")];
    let pinned_result = run_simple(&fixture, &pinned).await.unwrap();
    let result = run_simple(&fixture, &bare).await.unwrap();
    assert_eq!(picked(&pinned_result), "a");
    assert!(pinned_result.strategy.is_none());
    assert_eq!(picked(&result), "a");
    assert_eq!(result.strategy, Some(Strategy::SimpleShuffle));
}

pub(super) async fn deployment_id_single() {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let setup = RequestSetup::new("a");
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(result.group, "openai/a");
    assert_eq!(picked(&result), "a");
    assert!(result.strategy.is_none());
    assert_eq!(
        result.aliases,
        vec![AliasStep {
            source: AliasSource::DeploymentId,
            from: String::from("a"),
            to: String::from("openai/a"),
        }]
    );
}

pub(super) async fn excluded_drop() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 5), weighted("gpt-4o", "b", 0)]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.excluded = vec![String::from("a")];
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(picked(&result), "b");
}

pub(super) async fn retry_skip_narrow() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 5), weighted("gpt-4o", "b", 0)]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.retry_skipped = vec![String::from("a")];
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(picked(&result), "b");
}

pub(super) async fn retry_skip_keeps_pool() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 5), weighted("gpt-4o", "b", 0)]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.retry_skipped = vec![String::from("a"), String::from("b")];
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(picked(&result), "a");
}

fn aliased_fixture(group: &[(&str, &str)], model: &[(&str, &str)]) -> Fixture {
    Fixture::aliased(vec![deployment("gpt-4o", "a", &[])], group, model)
}

pub(super) async fn group_alias() {
    let fixture = aliased_fixture(&[("gpt", "gpt-4o")], &[]);
    let source = AliasSource::ModelGroupAlias;
    alias_resolves(&fixture, source, "gpt", "gpt-4o").await;
}

pub(super) async fn model_alias_map() {
    let fixture = aliased_fixture(&[], &[("gpt-4o", "gpt-4o-mini")]);
    let source = AliasSource::ModelAliasMap;
    alias_resolves(&fixture, source, "gpt-4o", "gpt-4o-mini").await;
}

async fn alias_resolves(fixture: &Fixture, source: AliasSource, requested: &str, resolved: &str) {
    let result = run_simple(fixture, &RequestSetup::new(requested))
        .await
        .unwrap();
    assert_eq!(result.group, resolved);
    assert_eq!(picked(&result), "a");
    assert_eq!(
        result.aliases,
        vec![AliasStep {
            source,
            from: String::from(requested),
            to: String::from(resolved),
        }]
    );
}

pub(super) async fn client_acquired() {
    let fixture = Fixture::plain(vec![deployment(
        "gpt-4o",
        "a",
        &[("api_base", text("https://example.test/v1"))],
    )]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.clients = Some(CachedClients::new(Duration::from_secs(60), clock()));
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(
        result.client,
        Some(CachedClient {
            model_id: String::from("a"),
            model_group: String::from("gpt-4o"),
            api_base: String::from("https://example.test/v1"),
            stream: false,
            credential: ClientCredential::Configured,
        })
    );
}

// The deployments the healthy view keeps for a group, which it answers as a pool.
fn healthy_pool_of(found: &HealthyDeployments) -> &[DeploymentSpec] {
    let Candidates::Pool(pool) = &found.healthy else {
        panic!("a group answers a pool");
    };
    pool
}

// The healthy view of a request that names the model, which the catalog answers.
async fn healthy_of(fixture: &Fixture, model: &str) -> HealthyDeployments {
    run_healthy(fixture, &RequestSetup::new(model))
        .await
        .unwrap()
}

// The fixture whose group alias "gpt" names the two-deployment pool "gpt-4o".
fn aliased_pool() -> Fixture {
    Fixture::aliased(
        group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]),
        &[("gpt", "gpt-4o")],
        &[],
    )
}

// the healthy view keeps the pool, the alias it resolved and every deployment the catalog holds
pub(super) async fn healthy_pool() {
    let fixture = aliased_pool();
    let found = healthy_of(&fixture, "gpt").await;
    assert_eq!(found.group, "gpt-4o");
    assert_eq!(healthy_pool_of(&found).len(), 2);
    assert_eq!(found.aliases.len(), 1);
}

pub(super) async fn healthy_single() {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let found = healthy_of(&fixture, "a").await;
    assert_eq!(found.group, "openai/a");
    assert_eq!(found.all.len(), 1);
    assert_eq!(found.aliases.len(), 1);
    assert!(matches!(&found.healthy, Candidates::Single(one) if one.id() == Some("a")));
}

// the healthy view drops what an admin paused, while the whole pool still counts it
pub(super) async fn healthy_blocked_pool() {
    let fixture = Fixture::plain(vec![
        paused("gpt-4o", "paused", &[]),
        deployment("gpt-4o", "live", &[]),
    ]);
    let found = run_healthy(&fixture, &RequestSetup::new("gpt-4o"))
        .await
        .unwrap();
    assert_eq!(found.all.len(), 2);
    let pool = healthy_pool_of(&found);
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].id(), Some("live"));
}

// The deployment a simple-shuffle route over the fixture picks.
async fn picked_from(fixture: &Fixture) -> String {
    let result = run_simple(fixture, &RequestSetup::new("gpt-4o"))
        .await
        .unwrap();
    picked(&result).to_owned()
}

// the lowest declared order narrows the pool before the strategy weighs what is left
pub(super) async fn order_lowest_wins() {
    let first = deployment("gpt-4o", "a", &[("order", number(1))]);
    let second = with_info(
        "gpt-4o",
        "b",
        &[("order", number(2)), ("weight", number(5))],
        &[],
    );
    let fixture = Fixture::plain(vec![second, first]);
    assert_eq!(picked_from(&fixture).await, "a");
}

// the order filter runs after the paused filter, so a paused order-1 leaves the live order-2 to serve
pub(super) async fn order_after_blocked() {
    let blocked = paused("gpt-4o", "a", &[("order", number(1))]);
    let live = deployment("gpt-4o", "b", &[("order", number(2))]);
    let fixture = Fixture::plain(vec![blocked, live]);
    assert_eq!(picked_from(&fixture).await, "b");
}

pub(super) fn headers_attach_to_object() {
    let mut prepared = prepare_response(body(&[("id", text("chatcmpl-1"))])).unwrap();
    assert!(!response_has_hidden_params(&prepared));
    ensure_additional_headers(&mut prepared).insert(String::from("x-probe"), text("1"));
    apply_response_model_id(&mut prepared, Some(&metadata("a")));
    assert!(response_has_hidden_params(&prepared));
    assert_eq!(
        write_hidden_params(prepared),
        body(&[
            ("id", text("chatcmpl-1")),
            (
                "_hidden_params",
                Json::Object(object(&[
                    ("model_id", text("a")),
                    (
                        "additional_headers",
                        Json::Object(object(&[("x-probe", text("1"))]))
                    ),
                ])),
            ),
        ])
    );
}

pub(super) fn header_model_id_kept() {
    let mut prepared = prepare_response(body(&[hidden(&[("model_id", text("b"))])])).unwrap();
    ensure_additional_headers(&mut prepared);
    apply_response_model_id(&mut prepared, Some(&metadata("a")));
    assert_eq!(
        write_hidden_params(prepared),
        body(&[(
            "_hidden_params",
            Json::Object(object(&[
                ("model_id", text("b")),
                ("additional_headers", Json::Object(JsonObject::new())),
            ])),
        ),])
    );
}

pub(super) fn token_count_from_usage() {
    let response = object(&[(
        "usage",
        Json::Object(object(&[("total_tokens", number(9))])),
    )]);
    assert_eq!(response_total_token_count(&response), 9);
}

pub(super) fn token_count_from_input_output() {
    let response = object(&[(
        "usage",
        Json::Object(object(&[
            ("total_tokens", number(0)),
            ("input_tokens", number(4)),
            ("output_tokens", number(5)),
        ])),
    )]);
    assert_eq!(response_total_token_count(&response), 9);
}

// The attempt a named deployment resolves to when the request carries its own routing_strategy.
async fn override_attempt(configured: RoutingStrategy, strategy: &str) -> AttemptResult {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let mut setup = RequestSetup::new("a");
    setup.routing_strategy = Some(strategy.to_owned());
    let mut random = seeded();
    run_route(&fixture, &setup, &NoServices, configured, &mut random)
        .await
        .unwrap()
}

// a request that names another strategy gets that selector's override pre-call check, bound as a callback
pub(super) async fn override_pre_call_bound() {
    let result = override_attempt(RoutingStrategy::SimpleShuffle, "usage-based-routing-v2").await;
    let target = result.override_pre_call.expect("the override selector");
    assert_eq!(result.bindings.dynamic_callbacks, vec![target]);
}

// an override naming the configured strategy reuses its default selector, which runs no override pre-call check
pub(super) async fn override_pre_call_absent() {
    let result = override_attempt(RoutingStrategy::LeastBusy, "least-busy").await;
    assert_eq!(result.override_pre_call, None);
    assert!(result.bindings.dynamic_callbacks.is_empty());
}

// the local count under one deployment, zero when the counter was never written
fn in_flight(service: &LeastBusy, key: &RouteKey) -> i64 {
    match service.local.get(key).unwrap() {
        StoreRead::Value(RouteValue::Counter(count)) => count,
        StoreRead::Missing => 0,
        StoreRead::Value(_) => panic!("the in-flight value is not a counter"),
    }
}

// get_available_deployments: the least counted deployment wins, and dropping the attempt returns this worker's count
pub(super) async fn least_busy_picks_lowest_count() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]));
    let service = Arc::new(LeastBusy {
        local: in_flight_store(),
        shared: None,
    });
    let busy_key = request_count_key("gpt-4o", "b").unwrap();
    service
        .local
        .set(
            &busy_key,
            &RouteValue::Counter(3),
            Some(IN_FLIGHT_COUNT_TTL_SECONDS),
        )
        .unwrap();
    let counted = Counted(Arc::clone(&service));
    let mut random = seeded();
    let setup = RequestSetup::new("gpt-4o");
    let mut result = run_route(
        &fixture,
        &setup,
        &counted,
        RoutingStrategy::LeastBusy,
        &mut random,
    )
    .await
    .unwrap();
    assert_eq!(picked(&result), "a");
    assert_eq!(result.strategy, Some(Strategy::LeastBusy));
    let picked_key = request_count_key("gpt-4o", "a").unwrap();
    assert_eq!(in_flight(&service, &picked_key), 1);
    let started = result
        .least_busy
        .take()
        .expect("the attempt holds its count");
    assert!(started.release().await.is_none());
    assert_eq!(in_flight(&service, &picked_key), 0);
    {
        let dropped = run_route(
            &fixture,
            &setup,
            &counted,
            RoutingStrategy::LeastBusy,
            &mut random,
        )
        .await
        .unwrap();
        assert_eq!(picked(&dropped), "a");
        assert_eq!(in_flight(&service, &picked_key), 1);
    }
    assert_eq!(in_flight(&service, &picked_key), 0);
}

// every route attempt holds an in-flight count while it runs, whatever strategy the config names
pub(super) async fn each_strategy_counts_its_attempt() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]));
    let service = Arc::new(LeastBusy {
        local: in_flight_store(),
        shared: None,
    });
    let counted = Counted(Arc::clone(&service));
    let mut random = seeded();
    let setup = RequestSetup::new("gpt-4o");
    let mut result = run_route(
        &fixture,
        &setup,
        &counted,
        RoutingStrategy::SimpleShuffle,
        &mut random,
    )
    .await
    .unwrap();
    let key = request_count_key("gpt-4o", picked(&result)).unwrap();
    assert_eq!(in_flight(&service, &key), 1);
    let started = result
        .least_busy
        .take()
        .expect("the attempt holds its count");
    assert!(started.release().await.is_none());
    assert_eq!(in_flight(&service, &key), 0);
}
