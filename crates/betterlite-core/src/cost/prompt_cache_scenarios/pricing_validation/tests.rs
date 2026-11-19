#![cfg(test)]

use super::selected_prices;
use crate::cost::prompt_cache_scenarios::test_support::DEPLOYMENT_ID;
use crate::cost::prompt_cache_scenarios::test_support::Lookup;
use crate::cost::prompt_cache_scenarios::test_support::MODEL;
use crate::cost::prompt_cache_scenarios::test_support::Prices;
use crate::cost::prompt_cache_scenarios::test_support::flat_tariff;
use crate::cost::prompt_cache_scenarios::test_support::number;
use crate::cost::prompt_cache_scenarios::test_support::record;
use crate::cost::prompt_cache_scenarios::test_support::rendered;
use crate::cost::prompt_cache_scenarios::test_support::text;

const MISSING_DEPLOYMENT_ID: &str = "deployment-missing";

#[test]
fn a_deployment_tariff_is_selected_by_its_router_id() {
    let tariff = flat_tariff();
    let prices = Prices::new(Lookup::new(&[(DEPLOYMENT_ID, tariff.clone())]));
    let environment = prices.environment();
    let selected =
        selected_prices(&environment, MODEL, DEPLOYMENT_ID).expect("the deployment tariff");
    assert_eq!(selected.0, DEPLOYMENT_ID);
    assert_eq!(rendered(&selected.1), rendered(&tariff));
}

#[test]
fn a_deployment_tariff_wins_over_the_catalog_entry() {
    let catalog = record(&[
        ("key", text(MODEL)),
        ("input_cost_per_token", number("0.5")),
    ]);
    let lookup = Lookup::new(&[(DEPLOYMENT_ID, flat_tariff())]).with_effective(MODEL, catalog);
    let prices = Prices::new(lookup);
    let environment = prices.environment();
    let selected =
        selected_prices(&environment, MODEL, DEPLOYMENT_ID).expect("the deployment tariff");
    assert_eq!(selected.0, DEPLOYMENT_ID);
    assert_eq!(rendered(&selected.1), rendered(&flat_tariff()));
}

#[test]
fn a_catalog_entry_with_its_key_is_selected_without_a_deployment_tariff() {
    let catalog = record(&[
        ("key", text(MODEL)),
        ("input_cost_per_token", number("0.002")),
    ]);
    let prices = Prices::new(Lookup::new(&[]).with_effective(MODEL, catalog.clone()));
    let environment = prices.environment();
    let selected =
        selected_prices(&environment, MODEL, MISSING_DEPLOYMENT_ID).expect("the catalog tariff");
    assert_eq!(selected.0, MODEL);
    assert_eq!(rendered(&selected.1), rendered(&catalog));
}

#[test]
fn a_catalog_entry_without_its_key_is_refused() {
    let catalog = record(&[("input_cost_per_token", number("0.002"))]);
    let prices = Prices::new(Lookup::new(&[]).with_effective(MODEL, catalog));
    let environment = prices.environment();
    assert!(selected_prices(&environment, MODEL, MISSING_DEPLOYMENT_ID).is_none());
}

#[test]
fn an_empty_catalog_entry_is_skipped() {
    let prices = Prices::new(Lookup::new(&[]).with_effective(MODEL, record(&[])));
    let environment = prices.environment();
    assert!(selected_prices(&environment, MODEL, MISSING_DEPLOYMENT_ID).is_none());
}

#[test]
fn an_unknown_model_and_deployment_has_no_prices() {
    let prices = Prices::new(Lookup::new(&[]));
    let environment = prices.environment();
    assert!(selected_prices(&environment, MODEL, MISSING_DEPLOYMENT_ID).is_none());
}

#[test]
fn a_zero_rate_tariff_is_still_selected() {
    let tariff = record(&[
        ("input_cost_per_token", number("0")),
        ("output_cost_per_token", number("0")),
    ]);
    let prices = Prices::new(Lookup::new(&[(DEPLOYMENT_ID, tariff.clone())]));
    let environment = prices.environment();
    let selected =
        selected_prices(&environment, MODEL, DEPLOYMENT_ID).expect("the deployment tariff");
    assert_eq!(selected.0, DEPLOYMENT_ID);
    assert_eq!(rendered(&selected.1), rendered(&tariff));
}
