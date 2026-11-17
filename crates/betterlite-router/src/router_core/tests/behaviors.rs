use std::time::Duration;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::support::{
    Fixture, LastOnly, RequestSetup, body, clock, deployment, group_and_ids, hidden, metadata,
    number, object, picked, run_healthy, run_route, run_simple, seeded, text, weighted,
};
use crate::deployment_catalog::types::{AliasSource, AliasStep, Candidates};
use crate::deployment_clients::clients::{CachedClient, CachedClients, ClientCredential};
use crate::router_core::headers::{
    apply_response_model_id, ensure_additional_headers, prepare_response,
    response_has_hidden_params, response_total_token_count, write_hidden_params,
};
use crate::strategy::Strategy;
use crate::strategy_dispatch::strategy::RoutingStrategy;

pub(super) async fn run(name: &str) {
    match name {
        "weighted_pick" => weighted_pick().await,
        "shared_pool_chat_and_embedding" => shared_pool_chat_and_embedding().await,
        "callback_narrowed" => callback_narrowed().await,
        "specific_deployment_pool" => specific_deployment_pool().await,
        "pinned_short_circuit" => pinned_short_circuit().await,
        "deployment_id_single" => deployment_id_single().await,
        "excluded_drop" => excluded_drop().await,
        "retry_skip_narrow" => retry_skip_narrow().await,
        "retry_skip_keeps_pool" => retry_skip_keeps_pool().await,
        "group_alias" => group_alias().await,
        "model_alias_map" => model_alias_map().await,
        "client_acquired" => client_acquired().await,
        "healthy_pool" => healthy_pool().await,
        "healthy_single" => healthy_single().await,
        "headers_attach_to_object" => headers_attach_to_object(),
        "header_model_id_kept" => header_model_id_kept(),
        "token_count_from_usage" => token_count_from_usage(),
        "token_count_from_input_output" => token_count_from_input_output(),
        _ => panic!("unknown case {name}"),
    }
}

async fn weighted_pick() {
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

async fn shared_pool_chat_and_embedding() {
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

async fn callback_narrowed() {
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

async fn specific_deployment_pool() {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let mut setup = RequestSetup::new("openai/a");
    setup.specific_deployment = true;
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(result.group, "openai/a");
    assert_eq!(picked(&result), "a");
    assert_eq!(result.strategy, Some(Strategy::SimpleShuffle));
    assert!(result.aliases.is_empty());
}

async fn pinned_short_circuit() {
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

async fn deployment_id_single() {
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

async fn excluded_drop() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 5), weighted("gpt-4o", "b", 0)]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.excluded = vec![String::from("a")];
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(picked(&result), "b");
}

async fn retry_skip_narrow() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 5), weighted("gpt-4o", "b", 0)]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.retry_skipped = vec![String::from("a")];
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(picked(&result), "b");
}

async fn retry_skip_keeps_pool() {
    let fixture = Fixture::plain(vec![weighted("gpt-4o", "a", 5), weighted("gpt-4o", "b", 0)]);
    let mut setup = RequestSetup::new("gpt-4o");
    setup.retry_skipped = vec![String::from("a"), String::from("b")];
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(picked(&result), "a");
}

async fn group_alias() {
    let fixture = Fixture::aliased(
        vec![deployment("gpt-4o", "a", &[])],
        &[("gpt", "gpt-4o")],
        &[],
    );
    let setup = RequestSetup::new("gpt");
    let result = run_simple(&fixture, &setup).await.unwrap();
    assert_eq!(result.group, "gpt-4o");
    assert_eq!(
        result.aliases,
        vec![AliasStep {
            source: AliasSource::ModelGroupAlias,
            from: String::from("gpt"),
            to: String::from("gpt-4o"),
        }]
    );
}

async fn model_alias_map() {
    let fixture = Fixture::aliased(
        vec![deployment("gpt-4o", "a", &[])],
        &[],
        &[("gpt-4o", "gpt-4o-mini")],
    );
    let result = run_simple(&fixture, &RequestSetup::new("gpt-4o"))
        .await
        .unwrap();
    assert_eq!(result.group, "gpt-4o-mini");
    assert_eq!(picked(&result), "a");
    assert_eq!(
        result.aliases,
        vec![AliasStep {
            source: AliasSource::ModelAliasMap,
            from: String::from("gpt-4o"),
            to: String::from("gpt-4o-mini"),
        }]
    );
}

async fn client_acquired() {
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

async fn healthy_pool() {
    let fixture = Fixture::plain(group_and_ids(&[("gpt-4o", "a"), ("gpt-4o", "b")]));
    let Ok(found) = run_healthy(&fixture, &RequestSetup::new("gpt-4o")).await else {
        panic!("the group resolves to its pool");
    };
    assert_eq!(found.group, "gpt-4o");
    assert_eq!(found.all.len(), 2);
    assert!(matches!(&found.healthy, Candidates::Pool(pool) if pool.len() == 2));
    assert!(found.aliases.is_empty());
}

async fn healthy_single() {
    let fixture = Fixture::plain(vec![deployment("gpt-4o", "a", &[])]);
    let setup = RequestSetup::new("a");
    let Ok(found) = run_healthy(&fixture, &setup).await else {
        panic!("a deployment id resolves to one deployment");
    };
    assert_eq!(found.group, "openai/a");
    assert_eq!(found.all.len(), 1);
    assert!(matches!(&found.healthy, Candidates::Single(one) if one.id() == Some("a")));
}

fn headers_attach_to_object() {
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

fn header_model_id_kept() {
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

fn token_count_from_usage() {
    let response = object(&[(
        "usage",
        Json::Object(object(&[("total_tokens", number(9))])),
    )]);
    assert_eq!(response_total_token_count(&response), 9);
}

fn token_count_from_input_output() {
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
