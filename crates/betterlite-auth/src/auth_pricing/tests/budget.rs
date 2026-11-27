use rstest::rstest;
use rust_decimal::Decimal;

use crate::auth_key_budgets::key_budget::BUDGET_EXCEEDED_STATUS;
use crate::auth_key_budgets::money::Money;
use crate::auth_pricing::request::global_proxy_budget_check;
use crate::auth_pricing::routes::{
    BUDGET_ENFORCED_SIDE_EFFECT_ROUTES, MODEL_DISCOVERY_ROUTES, request_skips_budget_checks,
    route_skips_budget_checks,
};
use crate::auth_request_models::RequestedModels;

use super::support::{FakeCatalog, money, number, object, text};

const CHAT: &str = "/v1/chat/completions";

fn dec(value: &str) -> Decimal {
    value.parse().expect("the test decimal parses")
}

fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

fn free_catalog() -> FakeCatalog {
    let mut catalog = FakeCatalog::default();
    catalog.group("free", Some(money("0")), Some(money("0")));
    catalog.deployed("free", object(&[]), object(&[("id", text("free-dep"))]));
    catalog.raw("free-dep", object(&[("input_cost_per_token", number("0"))]));
    catalog
}

#[test]
fn zero_negative_budget_disabled() {
    assert!(
        global_proxy_budget_check(money("0"), Some(dec("5")), false, CHAT).is_none(),
        "a zero max_budget disables the global check"
    );
    assert!(global_proxy_budget_check(money("-1"), Some(dec("5")), false, CHAT).is_none());
}

#[test]
fn nonfinite_budget_no_rejection() {
    for budget in [Money::Nan, Money::PositiveInfinity, Money::NegativeInfinity] {
        assert!(global_proxy_budget_check(budget, Some(dec("100")), false, CHAT).is_none());
    }
}

#[test]
fn missing_spend_skips() {
    assert!(global_proxy_budget_check(money("1"), None, false, CHAT).is_none());
    assert!(global_proxy_budget_check(money("1"), Some(dec("2")), true, CHAT).is_none());
}

#[test]
fn equal_budget_allowed() {
    assert!(
        global_proxy_budget_check(money("5"), Some(dec("5")), false, CHAT).is_none(),
        "spending exactly the ceiling passes"
    );
    assert!(global_proxy_budget_check(money("5"), Some(dec("5.0001")), false, CHAT).is_some());
}

#[test]
fn proxy_budget_error_fields() {
    let denial = global_proxy_budget_check(money("5"), Some(dec("10")), false, CHAT)
        .expect("a spend over a positive ceiling denies");
    assert_eq!(denial.current_cost, dec("10"));
    assert_eq!(denial.max_budget, dec("5"));
    assert_eq!(denial.entity_type, "proxy");
    assert!(denial.entity_id.is_none());
    assert_eq!(denial.status_code, BUDGET_EXCEEDED_STATUS);
    assert_eq!(
        denial.message,
        "Budget has been exceeded! Current cost: 10, Max budget: 5"
    );
}

#[test]
fn models_aliases_exempt() {
    for route in ["/v1/models", "/models"] {
        assert!(
            global_proxy_budget_check(money("1"), Some(dec("100")), false, route).is_none(),
            "the model discovery aliases skip the global budget"
        );
    }
}

#[rstest]
#[case::v1_models("/v1/models")]
#[case::models("/models")]
#[case::model_info("/model/info")]
#[case::v1_model_info("/v1/model/info")]
#[case::v2_model_info("/v2/model/info")]
#[case::model_group_info("/model_group/info")]
#[case::utils_model_info("/utils/model_info")]
fn each_discovery_route(#[case] route: &str) {
    assert!(MODEL_DISCOVERY_ROUTES.contains(&route));
    assert!(route_skips_budget_checks(route));
    assert!(request_skips_budget_checks(
        route,
        &RequestedModels::None,
        None
    ));
}

#[rstest]
#[case::health("/health")]
#[case::health_services("/health/services")]
#[case::health_test_connection("/health/test_connection")]
#[case::auto_router_test_routing("/auto_router/test_routing")]
fn each_side_effect_route(#[case] route: &str) {
    assert!(BUDGET_ENFORCED_SIDE_EFFECT_ROUTES.contains(&route));
    assert!(!route_skips_budget_checks(route));
    assert!(!request_skips_budget_checks(
        route,
        &RequestedModels::None,
        None
    ));
}

#[test]
fn other_non_llm_skips() {
    assert!(route_skips_budget_checks("/key/info"));
    assert!(request_skips_budget_checks(
        "/key/info",
        &RequestedModels::None,
        None
    ));
    assert!(!route_skips_budget_checks(CHAT));
    assert!(!request_skips_budget_checks(
        CHAT,
        &RequestedModels::None,
        None
    ));
}

#[test]
fn free_side_effect_exemption() {
    let catalog = free_catalog();
    assert!(
        !route_skips_budget_checks("/health"),
        "the route alone does not exempt a side-effect route"
    );
    assert!(
        request_skips_budget_checks("/health", &one("free"), Some(&catalog)),
        "zero cost still exempts the side-effect route"
    );
    assert!(!request_skips_budget_checks(
        "/health",
        &one("paid"),
        Some(&catalog)
    ));
}
