use std::collections::BTreeMap;

use betterlite_config::sections::quality_router as section;
use betterlite_prompts::message_content::Message;

use super::{
    CONFIG_INVALID, CONFIG_VALID, deployment, json, keyword, messages, ordered, priced, router,
    router_with, strings, table, text_settings,
};
use crate::quality_router::QualityRouter;
use crate::quality_router::decision::Decision;

fn pick(router: &QualityRouter, request: &[Message]) -> Option<String> {
    let mut metadata = None;
    router
        .pre_route(Some(request), &mut metadata)
        .unwrap()
        .map(|route| route.model)
}

#[test]
fn a_keyword_override_short_circuits_the_classifier() {
    // the empty mapping would fail any classification, so only the keyword path can route
    let router = router_with(
        &["cheap", "strong"],
        Some(BTreeMap::new()),
        None,
        &[
            keyword(deployment("cheap", 1), "code"),
            keyword(deployment("strong", 3), "code"),
        ],
    );
    let request = messages(r#"[{"role":"user","content":"please write Code"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "strong");
    assert_eq!(route.messages, request);
    assert_eq!(route.decision, Decision::keyword("qr", "strong", "code", 3));
    assert_eq!(
        metadata,
        Some(json(
            r#"{"quality_router_decision":{"router_model_name":"qr","routed_model":"strong","routed_via":"keyword","matched_keyword":"code","quality_tier":3,"complexity_tier":null}}"#
        ))
    );
}

#[test]
fn keyword_ties_take_quality_then_order_then_price_then_name() {
    let request = messages(r#"[{"role":"user","content":"k"}]"#);
    let by_quality = router(
        &["low", "high"],
        &[
            keyword(ordered(deployment("low", 1), 1), "k"),
            keyword(ordered(deployment("high", 3), 9), "k"),
        ],
    );
    assert_eq!(pick(&by_quality, &request).as_deref(), Some("high"));
    let by_order = router(
        &["pricier", "cheaper"],
        &[
            keyword(ordered(priced(deployment("pricier", 2), 5, 1), 1), "k"),
            keyword(ordered(priced(deployment("cheaper", 2), 1, 1), 2), "k"),
        ],
    );
    assert_eq!(pick(&by_order, &request).as_deref(), Some("pricier"));
    let by_price = router(
        &["pricier", "cheaper"],
        &[
            keyword(priced(deployment("pricier", 2), 5, 1), "k"),
            keyword(priced(deployment("cheaper", 2), 1, 1), "k"),
        ],
    );
    assert_eq!(pick(&by_price, &request).as_deref(), Some("cheaper"));
    let by_name = router(
        &["bbb", "aaa"],
        &[
            keyword(priced(deployment("bbb", 2), 1, 1), "k"),
            keyword(priced(deployment("aaa", 2), 1, 1), "k"),
        ],
    );
    assert_eq!(pick(&by_name, &request).as_deref(), Some("aaa"));
}

#[test]
fn an_exact_quality_tier_wins_before_rounding() {
    let router = router_with(
        &["one", "two", "three"],
        Some(table(&[("SIMPLE", 2)])),
        None,
        &[
            deployment("one", 1),
            deployment("two", 2),
            deployment("three", 3),
        ],
    );
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "two");
    let Decision::QualityTier {
        quality_tier,
        complexity_tier,
        signals,
        ..
    } = route.decision
    else {
        panic!("the tier path decides a quality_tier route");
    };
    assert_eq!(quality_tier, 2);
    assert_eq!(complexity_tier, "SIMPLE");
    assert_eq!(signals, strings(&["short (2 tokens)", "simple (hello)"]));
    assert_eq!(
        metadata,
        Some(json(
            r#"{"quality_router_decision":{"router_model_name":"qr","routed_model":"two","routed_via":"quality_tier","matched_keyword":null,"quality_tier":2,"complexity_tier":"SIMPLE"}}"#
        ))
    );
}

#[test]
fn a_missing_quality_tier_rounds_up_before_down() {
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    let up = router_with(
        &["one", "three"],
        Some(table(&[("SIMPLE", 2)])),
        None,
        &[deployment("one", 1), deployment("three", 3)],
    );
    assert_eq!(pick(&up, &request).as_deref(), Some("three"));
    let down = router_with(
        &["one"],
        Some(table(&[("SIMPLE", 2)])),
        None,
        &[deployment("one", 1)],
    );
    assert_eq!(pick(&down, &request).as_deref(), Some("one"));
}

#[test]
fn an_empty_tier_index_falls_back_to_the_default_model() {
    let router = router_with(&[], None, Some("fb"), &[]);
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "fb");
    assert!(matches!(
        route.decision,
        Decision::QualityTier {
            quality_tier: 1,
            ..
        }
    ));
    assert_eq!(
        metadata,
        Some(json(
            r#"{"quality_router_decision":{"router_model_name":"qr","routed_model":"fb","routed_via":"quality_tier","matched_keyword":null,"quality_tier":1,"complexity_tier":"SIMPLE"}}"#
        ))
    );
}

#[test]
fn a_constructor_default_model_wins_over_the_configured_one() {
    let config = section::RouterConfig {
        default_model: Some("configured".to_owned()),
        ..section::RouterConfig::default()
    };
    let router = QualityRouter::new("qr", Some(&config), Some("explicit"), &[]).unwrap();
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    assert_eq!(pick(&router, &request).as_deref(), Some("explicit"));
    let system_only = messages(r#"[{"role":"system","content":"sys"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&system_only), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "explicit");
}

#[test]
fn no_user_message_routes_to_the_default_model_without_a_stash() {
    let router = router_with(&["one"], None, Some("fb"), &[deployment("one", 1)]);
    let request = messages(r#"[{"role":"system","content":"sys"}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&request), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "fb");
    assert_eq!(route.messages, request);
    assert_eq!(route.decision, Decision::default_fallback("qr", "fb"));
    assert_eq!(metadata, None);
}

#[test]
fn only_text_parts_of_a_content_list_carry_the_prompt() {
    let router = router_with(
        &["one"],
        Some(table(&[("SIMPLE", 2)])),
        Some("fb"),
        &[keyword(deployment("one", 2), "code")],
    );
    let blocks = messages(
        r#"[{"role":"user","content":[{"type":"text","text":"please"},{"type":"image_url","image_url":{"url":"x"}},{"type":"text","text":"code"}]}]"#,
    );
    let mut metadata = None;
    let route = router
        .pre_route(Some(&blocks), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.decision, Decision::keyword("qr", "one", "code", 2));
    let blank = messages(r#"[{"role":"user","content":[{"type":"text","text":"  "}]}]"#);
    let mut metadata = None;
    let route = router
        .pre_route(Some(&blank), &mut metadata)
        .unwrap()
        .unwrap();
    assert_eq!(route.model, "fb");
    assert_eq!(route.decision, Decision::default_fallback("qr", "fb"));
}

#[test]
fn absent_or_empty_messages_leave_the_request_alone() {
    let router = router(&["one"], &[deployment("one", 1)]);
    let mut metadata = None;
    let empty: [Message; 0] = [];
    assert!(router.pre_route(None, &mut metadata).unwrap().is_none());
    assert!(
        router
            .pre_route(Some(&empty), &mut metadata)
            .unwrap()
            .is_none()
    );
    assert_eq!(metadata, None);
}

#[test]
fn unknown_keys_are_reported_and_the_route_still_runs() {
    let config = section::RouterConfig {
        available_models: vec!["one".to_owned()],
        extras: BTreeMap::from([("mystery".to_owned(), json("1"))]),
        ..section::RouterConfig::default()
    };
    let mut one = deployment("one", 1);
    if let Some(preferences) = one.preferences.as_mut() {
        preferences.extras.insert("odd".to_owned(), json(r#""x""#));
    }
    let router = QualityRouter::new("qr", Some(&config), None, &[one]).unwrap();
    assert_eq!(
        router.warnings,
        strings(&[
            "quality_router: ignoring unknown QualityRouterConfig field 'mystery' for router 'qr'",
            "quality_router: ignoring unknown RoutingPreferences field 'odd' for model 'one'",
        ])
    );
    let request = messages(r#"[{"role":"user","content":"hello there"}]"#);
    assert_eq!(pick(&router, &request).as_deref(), Some("one"));
}

#[test]
fn the_shipped_seeds_define_the_routers_document() {
    let settings = text_settings(CONFIG_VALID).unwrap();
    let config = settings.deployments[0].config.as_ref().unwrap();
    let router =
        QualityRouter::new("quality-router", Some(config), None, &settings.deployments).unwrap();
    assert!(router.warnings.is_empty());
    let request = messages(r#"[{"role":"user","content":"please write code"}]"#);
    assert_eq!(pick(&router, &request).as_deref(), Some("quality-fast"));
    assert!(text_settings(CONFIG_INVALID).is_err());
}
