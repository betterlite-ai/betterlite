use std::iter::once;
use std::time::Duration;

use betterlite_models::decisions::Json;

use super::support::{
    FixedChooser, FixedParams, Fixture, Seen, capture_logs, choice, fit, message, params, response,
};
use crate::complexity_scoring::decision::RoutingDecision;
use crate::complexity_scoring::tier::ComplexityTier;

#[tokio::test]
async fn a_disabled_route_never_predicts() {
    let mut fixture = Fixture::new();
    fixture.config.enabled = false;
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let original = response();
    let applied = fixture
        .apply(&chooser, Some(original.clone()), None)
        .await
        .unwrap();
    assert_eq!(applied, original);
    assert_eq!(chooser.calls(), 0);
    assert_eq!(fixture.params.calls(), 0);
}

#[tokio::test]
async fn a_response_without_a_decision_keeps_its_route() {
    let fixture = Fixture::new();
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let mut original = response();
    original.routing_decision = None;
    let applied = fixture
        .apply(&chooser, Some(original.clone()), None)
        .await
        .unwrap();
    assert_eq!(applied, original);
    assert_eq!(chooser.calls(), 0);
}

#[tokio::test]
async fn an_absent_response_stays_absent() {
    let fixture = Fixture::new();
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let applied = fixture.apply(&chooser, None, None).await;
    assert_eq!(applied, None);
    assert_eq!(chooser.calls(), 0);
}

#[tokio::test]
async fn a_declined_choice_keeps_the_classified_route() {
    let fixture = Fixture::new();
    let chooser = FixedChooser::new(None, Duration::ZERO);
    let original = response();
    let applied = fixture
        .apply(&chooser, Some(original.clone()), None)
        .await
        .unwrap();
    assert_eq!(applied, original);
    assert_eq!(chooser.calls(), 1);
}

#[tokio::test]
async fn a_choice_the_context_gate_rejects_keeps_the_classified_route() {
    let mut fixture = Fixture::new();
    fixture.fit = fit(&[("medium", Some(10))], Some(100));
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let original = response();
    let applied = fixture
        .apply(&chooser, Some(original.clone()), None)
        .await
        .unwrap();
    assert_eq!(applied, original);
    assert_eq!(chooser.calls(), 1);
    assert_eq!(fixture.params.calls(), 0);
}

#[tokio::test]
async fn a_warm_cheaper_model_replaces_the_route() {
    let mut fixture = Fixture::new();
    fixture
        .labels
        .insert(ComplexityTier::Medium, "Standard".to_owned());
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let applied = fixture
        .apply(&chooser, Some(response()), Some(&[message()]))
        .await
        .unwrap();
    assert_eq!(applied.model, "medium");
    assert_eq!(
        applied.litellm_params,
        Some(params(&[("max_tokens", 8192)]))
    );
    assert_eq!(applied.session_affinity_ttl_seconds, Some(3600));
    assert_eq!(applied.messages, Some(vec![message()]));
    assert_classified_keys_survive(applied.routing_decision.as_ref().unwrap());
}

// the classified decision's keys stay; the cache comparison's fields replace the routed ones
fn assert_classified_keys_survive(decision: &RoutingDecision) {
    assert_eq!(
        decision["router_model_name"],
        Json::Text("probe".to_owned())
    );
    assert_eq!(decision["routed_model"], Json::Text("medium".to_owned()));
    assert_eq!(
        decision["cause"],
        Json::Text("prompt_cache_cost".to_owned())
    );
    assert_eq!(decision["tier"], Json::Text("MEDIUM".to_owned()));
    assert_eq!(decision["tier_label"], Json::Text("Standard".to_owned()));
    assert_eq!(
        decision["tier_litellm_params"],
        Json::Object(params(&[("max_tokens", 8192)]))
    );
    assert_eq!(
        decision["signals"],
        Json::List(vec![
            Json::Text("short (2 tokens)".to_owned()),
            Json::Text("cache-aware:classified-model=small".to_owned()),
            Json::Text("cache-aware:classification-cause=heuristic_scorer".to_owned()),
            Json::Text("cache-aware:estimated-cost=0.00100000;original-cost=0.00400000".to_owned()),
        ])
    );
}

#[tokio::test]
async fn a_decision_without_signals_or_cause_starts_a_fresh_list() {
    let fixture = Fixture::new();
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let mut original = response();
    original.routing_decision =
        Some(once(("routed_model".to_owned(), Json::Text("small".to_owned()))).collect());
    let applied = fixture.apply(&chooser, Some(original), None).await.unwrap();
    let decision = applied.routing_decision.unwrap();
    assert_eq!(
        decision["signals"],
        Json::List(vec![
            Json::Text("cache-aware:classified-model=small".to_owned()),
            Json::Text("cache-aware:classification-cause=None".to_owned()),
            Json::Text("cache-aware:estimated-cost=0.00100000;original-cost=0.00400000".to_owned()),
        ])
    );
}

#[tokio::test]
async fn an_unlisted_tier_labels_itself() {
    let fixture = Fixture::new();
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    let decision = applied.routing_decision.unwrap();
    assert_eq!(decision["tier_label"], Json::Text("MEDIUM".to_owned()));
}

#[tokio::test]
async fn the_chooser_receives_the_config_response_and_messages() {
    let fixture = Fixture::new();
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let _applied = fixture
        .apply(&chooser, Some(response()), Some(&[message()]))
        .await
        .unwrap();
    assert_eq!(
        chooser.seen(),
        Some(Seen {
            model: "small".to_owned(),
            decision_present: true,
            output_tokens: 1024,
            messages_present: true,
            kwargs_keys: Vec::new(),
        })
    );
}

#[tokio::test]
async fn the_chooser_receives_the_request_kwargs() {
    let mut fixture = Fixture::new();
    fixture
        .kwargs
        .insert("temperature".to_owned(), Json::Bool(true));
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let _applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    assert_eq!(
        chooser.seen().map(|seen| seen.kwargs_keys),
        Some(vec!["temperature".to_owned()])
    );
}

#[tokio::test]
async fn the_choice_is_logged_with_both_costs() {
    let (captured, _guard) = capture_logs();
    let fixture = Fixture::new();
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let _applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    let messages = captured.messages();
    assert!(
        messages.contains(
            "cache-aware choice model=medium original=small estimated_cost=0.001 original_cost=0.004"
        ),
        "{messages}"
    );
}

#[tokio::test]
async fn a_tier_credential_is_masked_in_the_decision_only() {
    let mut fixture = Fixture::new();
    let mut tier_params = params(&[("max_tokens", 8192)]);
    tier_params.insert(
        "api_key".to_owned(),
        Json::Text("sk-live-secret".to_owned()),
    );
    fixture.params = FixedParams::new(tier_params);
    let chooser = FixedChooser::new(Some(choice("0.001", "0.004")), Duration::ZERO);
    let applied = fixture
        .apply(&chooser, Some(response()), None)
        .await
        .unwrap();
    let decision = applied.routing_decision.unwrap();
    let Json::Object(recorded) = &decision["tier_litellm_params"] else {
        panic!("tier_litellm_params is an object");
    };
    let unsecret = params(&[("max_tokens", 8192)]);
    assert_eq!(recorded["max_tokens"], unsecret["max_tokens"]);
    assert_ne!(recorded["api_key"], Json::Text("sk-live-secret".to_owned()));
    let Some(params) = applied.litellm_params else {
        panic!("the chosen model's params are stamped on the response");
    };
    assert_eq!(params["api_key"], Json::Text("sk-live-secret".to_owned()));
}
