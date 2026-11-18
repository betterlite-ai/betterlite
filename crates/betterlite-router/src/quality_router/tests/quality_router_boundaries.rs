use std::collections::BTreeMap;

use betterlite_config::sections::quality_router as section;
use betterlite_prompts::message_content::Message;
use rstest::rstest;

use super::{
    deployment, json, keyword, keyword_decision, messages, ordered, router, router_with, table,
    text_settings,
};
use crate::quality_router::QualityRouter;
use crate::quality_router::decision::Decision;
use crate::quality_router::error::QualityRouterError;

fn try_router(
    available: &[&str],
    deployments: &[section::Deployment],
) -> Result<QualityRouter, QualityRouterError> {
    let config = section::RouterConfig {
        available_models: available.iter().map(|name| (*name).to_owned()).collect(),
        default_model: None,
        complexity_to_quality: section::default_complexity_to_quality(),
        extras: BTreeMap::new(),
    };
    QualityRouter::new("qr", Some(&config), None, deployments)
}

fn error_for(router: &QualityRouter, request: &[Message]) -> QualityRouterError {
    let mut metadata = None;
    let Err(error) = router.pre_route(Some(request), &mut metadata) else {
        panic!("the request is expected to be refused");
    };
    error
}

#[test]
fn an_available_model_without_preferences_is_refused() {
    let bare = section::Deployment {
        model_name: "ghost".to_owned(),
        config: None,
        preferences: None,
        input_cost_per_token: None,
    };
    let Err(error) = try_router(&["ghost"], &[bare]) else {
        panic!("a model without routing preferences is refused");
    };
    assert!(matches!(
        error,
        QualityRouterError::MissingPreferences { model } if model == "ghost"
    ));
}

#[test]
fn an_available_model_outside_the_model_list_is_refused() {
    let Err(error) = try_router(&["ok", "ghost"], &[deployment("ok", 1)]) else {
        panic!("a missing available model is refused");
    };
    assert!(matches!(
        error,
        QualityRouterError::MissingModels { models } if models == vec!["ghost".to_owned()]
    ));
}

#[test]
fn a_complexity_tier_outside_the_mapping_is_refused() {
    let router = router_with(
        &["one"],
        Some(BTreeMap::new()),
        None,
        &[deployment("one", 1)],
    );
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    assert!(matches!(
        error_for(&router, &request),
        QualityRouterError::MissingComplexityMapping { tier } if tier == "SIMPLE"
    ));
}

#[test]
fn a_tier_with_no_model_and_no_default_model_is_refused() {
    let router = router_with(&[], None, None, &[]);
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    assert!(matches!(
        error_for(&router, &request),
        QualityRouterError::NoModelForTier { tier: 1 }
    ));
}

#[test]
fn a_request_without_a_user_message_and_without_a_default_model_is_refused() {
    let router = router_with(&[], None, None, &[]);
    let request = messages(r#"[{"role":"system","content":"sys"}]"#);
    assert!(matches!(
        error_for(&router, &request),
        QualityRouterError::NoUserMessage
    ));
}

#[test]
fn a_text_part_whose_text_is_not_a_string_is_refused() {
    let router = router_with(&["one"], None, None, &[deployment("one", 1)]);
    let request = messages(r#"[{"role":"user","content":[{"type":"text","text":5}]}]"#);
    assert!(matches!(
        error_for(&router, &request),
        QualityRouterError::PartTextNotString { role, part } if role == "user" && part == 0
    ));
}

#[rstest]
#[case::rounds_down_from_i64_max(i64::MAX, i64::MAX - 1)]
#[case::rounds_up_from_i64_min(i64::MIN, i64::MIN + 1)]
#[case::takes_an_exact_zero(0, 0)]
fn the_tier_index_bounds_round_without_overflowing(
    #[case] quality_tier: i64,
    #[case] model_tier: i64,
) {
    let router = router_with(
        &["one"],
        Some(table(&[("SIMPLE", quality_tier)])),
        None,
        &[deployment("one", model_tier)],
    );
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "one");
}

#[test]
fn an_empty_keyword_never_matches() {
    let router = router_with(
        &["one"],
        Some(table(&[("SIMPLE", 1)])),
        None,
        &[keyword(deployment("one", 1), "")],
    );
    let request = messages(r#"[{"role":"user","content":"anything at all"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert!(matches!(route.decision, Decision::QualityTier { .. }));
}

#[test]
fn keyword_matching_ignores_case_in_the_message_and_the_keyword() {
    let router = router_with(
        &["one"],
        Some(table(&[("SIMPLE", 1)])),
        None,
        &[keyword(deployment("one", 1), "CoDe")],
    );
    let request = messages(r#"[{"role":"user","content":"WRITE CODE"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.decision, keyword_decision("qr", "one", "code", 1));
}

// the kept bug docs/deviations/B338.yaml records: a keyword also matches inside a longer word
#[test]
fn a_keyword_matches_inside_a_longer_word() {
    let router = router_with(
        &["one"],
        Some(table(&[("SIMPLE", 1)])),
        None,
        &[keyword(deployment("one", 1), "code")],
    );
    let request = messages(r#"[{"role":"user","content":"please encode this"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.decision, keyword_decision("qr", "one", "code", 1));
}

#[test]
fn a_metadata_object_without_a_decision_gains_the_routers_decision() {
    let router = router_with(
        &["one"],
        Some(table(&[("SIMPLE", 1)])),
        None,
        &[keyword(deployment("one", 1), "k")],
    );
    let request = messages(r#"[{"role":"user","content":"k"}]"#);
    let mut metadata = Some(json(r#"{"trace_id":"t"}"#));
    router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(
        metadata,
        Some(json(
            r#"{"trace_id":"t","quality_router_decision":{"router_model_name":"qr","routed_model":"one","routed_via":"keyword","matched_keyword":"k","quality_tier":1,"complexity_tier":null}}"#
        ))
    );
}

#[test]
fn an_existing_decision_in_the_metadata_is_replaced() {
    let router = router_with(
        &["one"],
        Some(table(&[("SIMPLE", 1)])),
        None,
        &[keyword(deployment("one", 1), "k")],
    );
    let request = messages(r#"[{"role":"user","content":"k"}]"#);
    let mut metadata = Some(json(r#"{"quality_router_decision":{"kept":true}}"#));
    router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(
        metadata,
        Some(json(
            r#"{"quality_router_decision":{"router_model_name":"qr","routed_model":"one","routed_via":"keyword","matched_keyword":"k","quality_tier":1,"complexity_tier":null}}"#
        ))
    );
}

#[test]
fn a_metadata_value_that_is_not_an_object_is_left_alone() {
    let router = router_with(
        &["one"],
        Some(table(&[("SIMPLE", 1)])),
        None,
        &[deployment("one", 1)],
    );
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    let mut metadata = Some(json(r#"["kept"]"#));
    router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(metadata, Some(json(r#"["kept"]"#)));
}

#[test]
fn a_name_declared_twice_is_sorted_by_its_last_preferences() {
    let router = router_with(
        &["dup", "other"],
        Some(table(&[("SIMPLE", 2)])),
        None,
        &[
            ordered(deployment("dup", 2), 1),
            ordered(deployment("dup", 2), 5),
            ordered(deployment("other", 2), 2),
        ],
    );
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "other");
    assert!(matches!(route.decision, Decision::QualityTier { .. }));
}

// both orders become the same f64 in LiteLLM, so its name tiebreak picks aaa; the exact i64 keeps the order
#[test]
fn large_orders_keep_more_precision_than_python_floats() {
    let router = router(
        &["aaa", "zzz"],
        &[
            ordered(keyword(deployment("aaa", 2), "k"), 9_007_199_254_740_993),
            ordered(keyword(deployment("zzz", 2), "k"), 9_007_199_254_740_992),
        ],
    );
    let request = messages(r#"[{"role":"user","content":"k"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "zzz");
}

const DECIMAL_PRICE_CONFIG: &str = "\
model_list:
  - model_name: qr
    litellm_params:
      quality_router_config:
        available_models: [aaa, zzz]
        complexity_to_quality: {SIMPLE: 2}
  - model_name: aaa
    litellm_params:
      model: openai/aaa
    model_info:
      litellm_routing_preferences: {quality_tier: 2}
      input_cost_per_token: \"0.1000000000000000001\"
  - model_name: zzz
    litellm_params:
      model: openai/zzz
    model_info:
      litellm_routing_preferences: {quality_tier: 2}
      input_cost_per_token: \"0.1\"
";

// both string prices become the same float in LiteLLM, so its name tiebreak picks aaa; the loaded Decimal keeps zzz cheaper
#[test]
fn decimal_prices_keep_digits_past_float_precision() {
    let settings = text_settings(DECIMAL_PRICE_CONFIG).unwrap();
    let aaa = settings
        .deployments
        .iter()
        .find(|deployment| deployment.model_name == "aaa")
        .unwrap();
    assert_eq!(
        aaa.input_cost_per_token,
        Some("0.1000000000000000001".parse().unwrap())
    );
    let zzz = settings
        .deployments
        .iter()
        .find(|deployment| deployment.model_name == "zzz")
        .unwrap();
    assert_eq!(zzz.input_cost_per_token, Some("0.1".parse().unwrap()));
    let config = settings.deployments[0].config.as_ref().unwrap();
    let router = QualityRouter::new("qr", Some(config), None, &settings.deployments).unwrap();
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "zzz");
}

#[test]
fn a_non_finite_score_is_refused_by_the_record() {
    let decision = Decision::QualityTier {
        router_model_name: "qr".to_owned(),
        routed_model: "one".to_owned(),
        quality_tier: 1,
        complexity_tier: "SIMPLE".to_owned(),
        score: f64::NAN,
        signals: Vec::new(),
    };
    assert!(matches!(
        decision.record(),
        Err(QualityRouterError::NonFiniteScore)
    ));
}
