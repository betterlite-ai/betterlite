#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_core::shadow_evidence::cost::PriceMap;
use betterlite_core::shadow_evidence::failure::Location;
use betterlite_core::shadow_evidence::records::{CallFailure, ShadowResponse};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::call_router_shadow;
use crate::judge_targets::router::JudgeRouter;
use crate::shadow_calls::{
    ShadowCall, ShadowDeps, ShadowRequest, ShadowRouter, ShadowRouterError, ShadowRouterProvider,
};

#[derive(Clone)]
struct RecordedCall {
    model: String,
    messages: Vec<Json>,
    metadata: IndexMap<String, Json>,
    num_retries: u32,
    fallbacks: Vec<Json>,
    params: IndexMap<String, Json>,
}

struct RouterFake {
    response: Option<Json>,
    decision: Option<Json>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl RouterFake {
    fn reply(response: Json) -> Self {
        Self {
            response: Some(response),
            decision: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            response: None,
            decision: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn writing(self, decision: Json) -> Self {
        Self {
            decision: Some(decision),
            ..self
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl ShadowRouter for RouterFake {
    async fn acompletion(&self, call: ShadowCall<'_>) -> Result<Json, ShadowRouterError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCall {
                model: call.model.to_owned(),
                messages: call.messages.clone(),
                metadata: call.metadata.clone(),
                num_retries: call.num_retries,
                fallbacks: call.fallbacks.clone(),
                params: call.params.clone(),
            });
        if let Some(decision) = &self.decision {
            call.metadata
                .insert("routing_decision".to_owned(), decision.clone());
        }
        self.response.clone().ok_or_else(|| ShadowRouterError {
            class: "RouterError".to_owned(),
            location: Some(Location {
                file: "/srv/app/router.py",
                line: 42,
            }),
            message: "provider exploded".to_owned(),
        })
    }
}

struct Provider {
    shadow: Option<Arc<dyn ShadowRouter>>,
}

impl ShadowRouterProvider for Provider {
    fn shadow_router(&self) -> Option<Arc<dyn ShadowRouter>> {
        self.shadow.clone()
    }

    fn judge_router(&self) -> Option<Arc<dyn JudgeRouter>> {
        None
    }
}

struct Prices(Decimal);

impl PriceMap for Prices {
    fn completion_cost(&self, _response: &Json) -> Option<Decimal> {
        Some(self.0)
    }
}

struct Harness {
    messages: Vec<Json>,
    params: IndexMap<String, Json>,
    metadata: IndexMap<String, Json>,
    provider: Provider,
    prices: Prices,
}

impl Harness {
    fn new(fake: Option<Arc<RouterFake>>, priced: Decimal) -> Self {
        Self {
            messages: messages(),
            params: shadow_params(),
            metadata: parent_metadata(),
            provider: Provider {
                shadow: fake.map(|router| erased(&router)),
            },
            prices: Prices(priced),
        }
    }

    async fn call(&self) -> Result<ShadowResponse, CallFailure> {
        let request = ShadowRequest {
            target_model: "shadow-target",
            messages: &self.messages,
            shadow_params: &self.params,
            parent_metadata: &self.metadata,
        };
        call_router_shadow(
            &request,
            &ShadowDeps {
                provider: &self.provider,
                prices: &self.prices,
            },
        )
        .await
    }
}

fn erased(fake: &Arc<RouterFake>) -> Arc<dyn ShadowRouter> {
    Arc::<RouterFake>::clone(fake)
}

fn object<const SIZE: usize>(entries: [(&str, Json); SIZE]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn number(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn messages() -> Vec<Json> {
    vec![object([
        ("role", Json::Text("user".to_owned())),
        ("content", Json::Text("hello".to_owned())),
    ])]
}

fn parent_metadata() -> IndexMap<String, Json> {
    IndexMap::from([
        (
            "user_api_key".to_owned(),
            Json::Text("sk-shadowed".to_owned()),
        ),
        (
            "user_api_key_team_id".to_owned(),
            Json::Text("team-1".to_owned()),
        ),
        (
            "route".to_owned(),
            Json::Text("/v1/chat/completions".to_owned()),
        ),
        (
            "routing_decision".to_owned(),
            object([
                ("routed_model", Json::Text("real-model".to_owned())),
                ("tier", Json::Text("control".to_owned())),
            ]),
        ),
        (
            "user_api_key_budget_reservation".to_owned(),
            Json::Bool(true),
        ),
        (
            "user_api_key_auth".to_owned(),
            object([
                ("team_id", Json::Text("team-1".to_owned())),
                ("budget_reservation", Json::Bool(true)),
            ]),
        ),
    ])
}

fn shadow_params() -> IndexMap<String, Json> {
    IndexMap::from([
        ("temperature".to_owned(), number("0.2")),
        (
            "tools".to_owned(),
            Json::List(vec![object([
                ("type", Json::Text("function".to_owned())),
                ("function", Json::Text("tool".to_owned())),
            ])]),
        ),
    ])
}

fn text_response(text: &str, model: &str) -> Json {
    Json::Object(IndexMap::from([
        ("model".to_owned(), Json::Text(model.to_owned())),
        (
            "choices".to_owned(),
            Json::List(vec![Json::Object(IndexMap::from([
                (
                    "message".to_owned(),
                    object([("content", Json::Text(text.to_owned()))]),
                ),
                ("finish_reason".to_owned(), Json::Text("stop".to_owned())),
            ]))]),
        ),
    ]))
}

fn stamped(mut response: Json, cost: &str) -> Json {
    if let Json::Object(fields) = &mut response {
        fields.insert(
            "_hidden_params".to_owned(),
            object([("response_cost", number(cost))]),
        );
    }
    response
}

#[tokio::test]
async fn a_pod_without_a_router_fails_with_the_fixed_error() {
    let harness = Harness::new(None, Decimal::ZERO);

    let failure = harness.call().await.unwrap_err();

    assert_eq!(failure.error, "no router configured on this pod");
    assert_eq!(failure.cost, Decimal::ZERO);
    assert_eq!(failure.classifier_cost, Decimal::ZERO);
}

#[tokio::test]
async fn the_router_gets_the_target_model_the_messages_zero_retries_no_fallbacks_and_every_param() {
    let fake = Arc::new(RouterFake::reply(text_response(
        "shadow reply",
        "shadow-served",
    )));
    let harness = Harness::new(Some(Arc::clone(&fake)), Decimal::ZERO);

    let shadow = harness.call().await.unwrap();

    let calls = fake.calls();
    let call = calls.first().unwrap();
    assert_eq!(call.model, "shadow-target");
    assert_eq!(call.messages, messages());
    assert_eq!(call.num_retries, 0);
    assert!(call.fallbacks.is_empty());
    assert_eq!(call.params, shadow_params());
    assert_eq!(shadow.text, "shadow reply");
}

#[tokio::test]
async fn the_router_gets_detached_identity_metadata_and_leaves_the_parent_alone() {
    let fake = Arc::new(RouterFake::reply(text_response(
        "shadow reply",
        "shadow-served",
    )));
    let harness = Harness::new(Some(Arc::clone(&fake)), Decimal::ZERO);
    let before = harness.metadata.clone();

    harness.call().await.unwrap();

    let expected = IndexMap::from([
        (
            "user_api_key".to_owned(),
            Json::Text("sk-shadowed".to_owned()),
        ),
        (
            "user_api_key_team_id".to_owned(),
            Json::Text("team-1".to_owned()),
        ),
        (
            "user_api_key_auth".to_owned(),
            object([("team_id", Json::Text("team-1".to_owned()))]),
        ),
        (
            "internal_call_origin".to_owned(),
            Json::Text("shadow_eval_router".to_owned()),
        ),
    ]);
    assert_eq!(fake.calls().first().unwrap().metadata, expected);
    assert_eq!(harness.metadata, before);
}

#[tokio::test]
async fn a_router_error_names_the_error_and_keeps_the_classifier_cost_it_already_billed() {
    let decision = object([
        ("routed_model", Json::Text("routed".to_owned())),
        ("classifier_cost", number("0.25")),
    ]);
    let fake = Arc::new(RouterFake::failing().writing(decision));
    let harness = Harness::new(Some(fake), Decimal::ZERO);

    let failure = harness.call().await.unwrap_err();

    assert_eq!(
        failure.error,
        "shadow router call failed: RouterError at router.py:42: provider exploded"
    );
    assert_eq!(failure.cost, Decimal::ZERO);
    assert_eq!(failure.classifier_cost, Decimal::new(25, 2));
}

#[tokio::test]
async fn an_empty_reply_keeps_both_costs_and_names_the_served_model() {
    let fake = Arc::new(
        RouterFake::reply(stamped(text_response("", "shadow-served"), "0.75"))
            .writing(object([("classifier_cost", number("0.5"))])),
    );
    let harness = Harness::new(Some(fake), Decimal::ZERO);

    let failure = harness.call().await.unwrap_err();

    assert_eq!(
        failure.error,
        "shadow router returned an empty response; finish_reason=stop, model=shadow-served"
    );
    assert_eq!(failure.cost, Decimal::new(75, 2));
    assert_eq!(failure.classifier_cost, Decimal::new(5, 1));
}

#[tokio::test]
async fn the_response_model_wins_over_the_routed_model() {
    let fake = Arc::new(
        RouterFake::reply(text_response("text", "served"))
            .writing(object([("routed_model", Json::Text("routed".to_owned()))])),
    );

    assert_eq!(
        Harness::new(Some(fake), Decimal::ZERO)
            .call()
            .await
            .unwrap()
            .model,
        "served"
    );
}

#[tokio::test]
async fn the_routed_model_fills_in_when_the_response_names_no_model() {
    let fake = Arc::new(
        RouterFake::reply(text_response("text", ""))
            .writing(object([("routed_model", Json::Text("routed".to_owned()))])),
    );

    assert_eq!(
        Harness::new(Some(fake), Decimal::ZERO)
            .call()
            .await
            .unwrap()
            .model,
        "routed"
    );
}

#[tokio::test]
async fn without_a_response_model_or_a_decision_the_model_is_empty() {
    let fake = Arc::new(RouterFake::reply(text_response("text", "")));

    let shadow = Harness::new(Some(fake), Decimal::ZERO)
        .call()
        .await
        .unwrap();

    assert_eq!(shadow.model, "");
    assert_eq!(shadow.tier, None);
}

#[tokio::test]
async fn the_tier_comes_from_the_routing_decision_the_router_wrote_back() {
    let fake = Arc::new(
        RouterFake::reply(text_response("text", "served"))
            .writing(object([("tier_label", Json::Text("premium".to_owned()))])),
    );

    let shadow = Harness::new(Some(fake), Decimal::ZERO)
        .call()
        .await
        .unwrap();

    assert_eq!(shadow.tier.as_deref(), Some("premium"));
}

#[tokio::test]
async fn the_billed_cost_falls_back_to_the_price_map() {
    let fake = Arc::new(RouterFake::reply(text_response("text", "served")));

    let shadow = Harness::new(Some(fake), Decimal::new(125, 3))
        .call()
        .await
        .unwrap();

    assert_eq!(shadow.cost, Decimal::new(125, 3));
    assert_eq!(shadow.classifier_cost, Decimal::ZERO);
}

#[tokio::test]
async fn a_reply_keeps_the_billed_cost_and_the_classifier_cost_the_router_wrote_back() {
    let fake = Arc::new(
        RouterFake::reply(stamped(text_response("text", "served"), "0.75"))
            .writing(object([("classifier_cost", number("0.5"))])),
    );

    let shadow = Harness::new(Some(fake), Decimal::ZERO)
        .call()
        .await
        .unwrap();

    assert_eq!(shadow.cost, Decimal::new(75, 2));
    assert_eq!(shadow.classifier_cost, Decimal::new(5, 1));
}

const TOOL_CALL_RESPONSE: &str = r#"{
    "model": "served",
    "choices": [{
        "message": {
            "content": null,
            "tool_calls": [{"function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}}]
        },
        "finish_reason": "tool_calls"
    }]
}"#;

#[tokio::test]
async fn a_tool_call_is_the_reply_text_when_no_prose_was_written() {
    let fake = Arc::new(RouterFake::reply(
        serde_json::from_str(TOOL_CALL_RESPONSE).unwrap(),
    ));

    let shadow = Harness::new(Some(fake), Decimal::ZERO)
        .call()
        .await
        .unwrap();

    assert_eq!(shadow.text, "[tool call] get_weather({\"city\":\"Paris\"})");
}
