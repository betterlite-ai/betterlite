use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::auth_key_budgets::money::Money;
use crate::auth_pricing::cache::ZeroCostCache;
use crate::auth_pricing::zero_cost::is_model_cost_zero;
use crate::auth_request_models::RequestedModels;

use super::support::{FakeCatalog, captured, money, number, object, text};

fn one(model: &str) -> RequestedModels {
    RequestedModels::One(model.to_owned())
}

fn many(models: &[&str]) -> RequestedModels {
    RequestedModels::Many(models.iter().map(|model| (*model).to_owned()).collect())
}

// a deployment whose raw cost-map entry carries an explicit token price key
fn explicit_free(catalog: &mut FakeCatalog, model: &str) {
    catalog.group(model, Some(money("0")), Some(money("0")));
    let deployment_id = format!("{model}-dep");
    catalog.deployed(model, object(&[]), object(&[("id", text(&deployment_id))]));
    catalog.raw(
        &deployment_id,
        object(&[("input_cost_per_token", number("0"))]),
    );
}

#[test]
fn no_model_or_router_false() {
    let mut catalog = FakeCatalog::with_cache();
    explicit_free(&mut catalog, "free");
    assert!(!is_model_cost_zero(&RequestedModels::None, Some(&catalog)));
    assert!(!is_model_cost_zero(&one("free"), None));
    assert!(catalog.cached("free").is_none());
    assert!(catalog.lookups().is_empty());
}

#[test]
fn empty_list_free() {
    let catalog = FakeCatalog::with_cache();
    assert!(is_model_cost_zero(
        &RequestedModels::Many(Vec::new()),
        Some(&catalog)
    ));
    assert!(catalog.lookups().is_empty());
}

#[test]
fn cache_true_false() {
    let mut catalog = FakeCatalog::with_cache();
    explicit_free(&mut catalog, "free");
    assert!(is_model_cost_zero(&one("free"), Some(&catalog)));
    assert_eq!(catalog.cached("free"), Some(true));
    let lookups_after_first = catalog.lookups().len();
    assert!(is_model_cost_zero(&one("free"), Some(&catalog)));
    assert_eq!(
        catalog.lookups().len(),
        lookups_after_first,
        "a cached True skips the lookup"
    );

    catalog.group("paid", Some(money("0.5")), Some(money("0")));
    assert!(!is_model_cost_zero(&one("paid"), Some(&catalog)));
    assert_eq!(catalog.cached("paid"), Some(false));
    let lookups_after_miss = catalog.lookups().len();
    catalog.groups.remove("paid");
    assert!(!is_model_cost_zero(&one("paid"), Some(&catalog)));
    assert_eq!(
        catalog.lookups().len(),
        lookups_after_miss,
        "a cached False exits without a lookup"
    );
}

#[test]
fn nondict_cache_ignored() {
    let mut catalog = FakeCatalog::default();
    assert!(catalog.cache.is_none());
    explicit_free(&mut catalog, "free");
    assert!(is_model_cost_zero(&one("free"), Some(&catalog)));
    let lookups_after_first = catalog.lookups().len();
    assert!(is_model_cost_zero(&one("free"), Some(&catalog)));
    assert_eq!(
        catalog.lookups().len(),
        2 * lookups_after_first,
        "without a dict cache every call looks the model up again"
    );
}

#[test]
fn missing_group_debug_false() {
    let catalog = FakeCatalog::with_cache();
    let (verdict, logs) = captured(|| is_model_cost_zero(&one("ghost"), Some(&catalog)));
    assert!(!verdict);
    assert!(
        logs.iter()
            .any(|line| line == "No model group info found for ghost, assuming it has cost")
    );
    assert_eq!(catalog.cached("ghost"), Some(false));
}

#[test]
fn undefined_price_false() {
    let mut catalog = FakeCatalog::with_cache();
    catalog.group("half", Some(money("0")), None);
    let (verdict, logs) = captured(|| is_model_cost_zero(&one("half"), Some(&catalog)));
    assert!(!verdict);
    assert!(logs.iter().any(|line| line
        == "Model half has undefined cost (input: 0, output: None), assuming it has cost"));
    assert_eq!(catalog.cached("half"), Some(false));
}

#[test]
fn positive_price_false() {
    let mut catalog = FakeCatalog::with_cache();
    catalog.group("paid-input", Some(money("0.000001")), Some(money("0")));
    catalog.group("paid-output", Some(money("0")), Some(money("0.5")));
    let (input_verdict, input_logs) =
        captured(|| is_model_cost_zero(&one("paid-input"), Some(&catalog)));
    let (output_verdict, output_logs) =
        captured(|| is_model_cost_zero(&one("paid-output"), Some(&catalog)));
    assert!(!input_verdict);
    assert!(!output_verdict);
    assert!(input_logs.iter().any(|line| line
        == "Model paid-input has non-zero or non-finite cost (input: 0.000001, output: 0)"));
    assert!(
        output_logs.iter().any(|line| line
            == "Model paid-output has non-zero or non-finite cost (input: 0, output: 0.5)")
    );
    assert_eq!(catalog.cached("paid-input"), Some(false));
    assert_eq!(catalog.cached("paid-output"), Some(false));
}

#[test]
fn explicit_raw_key_required() {
    let mut catalog = FakeCatalog::with_cache();
    catalog.group("m", Some(money("0")), Some(money("0")));
    catalog.deployed("m", object(&[]), object(&[("id", text("m-dep"))]));
    catalog.deployed("m", object(&[]), object(&[]));
    // a sparse auto-registration entry carries the deployment id and no token price
    catalog.deployed("m", object(&[]), object(&[("id", text("m-sparse"))]));
    catalog.raw("m-sparse", object(&[("id", text("m-sparse"))]));
    let (verdict, logs) = captured(|| is_model_cost_zero(&one("m"), Some(&catalog)));
    assert!(
        !verdict,
        "an absent or sparse token-price key is not a free model"
    );
    assert!(logs.iter().any(|line| {
        line.contains(
            "Model m has zero cost but no explicit cost configuration in model_cost entry",
        )
    }));
    assert_eq!(catalog.cached("m"), Some(false));
}

#[test]
fn aliases_same_deployments() {
    let mut catalog = FakeCatalog::with_cache();
    explicit_free(&mut catalog, "fast-alias");
    assert!(is_model_cost_zero(&one("fast-alias"), Some(&catalog)));
    let lookups = catalog.lookups();
    assert_eq!(
        lookups,
        vec![
            "group:fast-alias".to_owned(),
            "list:fast-alias".to_owned(),
            "list:fast-alias".to_owned(),
        ],
        "both checks resolve the alias through the same catalog lookups"
    );
}

#[test]
fn null_raw_key_still_explicit() {
    let mut catalog = FakeCatalog::with_cache();
    catalog.group("null-input", Some(money("0")), Some(money("0")));
    catalog.deployed(
        "null-input",
        object(&[]),
        object(&[("id", text("null-input-dep"))]),
    );
    catalog.raw(
        "null-input-dep",
        object(&[("input_cost_per_token", Json::Null)]),
    );
    catalog.group("null-output", Some(money("0")), Some(money("0")));
    catalog.deployed(
        "null-output",
        object(&[]),
        object(&[("id", text("null-output-dep"))]),
    );
    catalog.raw(
        "null-output-dep",
        object(&[("output_cost_per_token", Json::Null)]),
    );
    assert!(is_model_cost_zero(&one("null-input"), Some(&catalog)));
    assert!(is_model_cost_zero(&one("null-output"), Some(&catalog)));
    assert_eq!(catalog.cached("null-input"), Some(true));
    assert_eq!(catalog.cached("null-output"), Some(true));
}

#[test]
fn ptu_zero_values_not_free() {
    let mut catalog = FakeCatalog::with_cache();
    explicit_free(&mut catalog, "ptu-zero");
    catalog.deployed(
        "ptu-zero",
        object(&[]),
        object(&[
            ("id", text("ptu-zero-capacity")),
            ("ptu_count", number("0")),
            ("cost_per_ptu_per_hour", number("0")),
        ]),
    );
    let (verdict, logs) = captured(|| is_model_cost_zero(&one("ptu-zero"), Some(&catalog)));
    assert!(!verdict, "zero-valued PTU fields still bill flat capacity");
    assert!(logs.iter().any(|line| line.contains(
        "Model ptu-zero prices reserved PTU capacity as a flat cost"
    )));
    assert_eq!(catalog.cached("ptu-zero"), Some(false));

    let mut partial = FakeCatalog::with_cache();
    explicit_free(&mut partial, "ptu-partial");
    partial.deployed(
        "ptu-partial",
        object(&[]),
        object(&[
            ("id", text("ptu-partial-capacity")),
            ("ptu_count", number("1")),
        ]),
    );
    assert!(
        is_model_cost_zero(&one("ptu-partial"), Some(&partial)),
        "one PTU field alone is not a flat cost"
    );
}

#[rstest]
#[case::negative("negative", money("-0.001"), money("0"), true, None)]
#[case::nan("nan", Money::Nan, money("0"), false, Some("input: NaN"))]
#[case::positive_infinity(
    "positive-inf",
    money("0"),
    Money::PositiveInfinity,
    false,
    Some("output: Infinity")
)]
#[case::negative_infinity(
    "negative-inf",
    Money::NegativeInfinity,
    money("0"),
    false,
    Some("input: -Infinity")
)]
fn negative_nonfinite_price_edges(
    #[case] model: &str,
    #[case] input: Money,
    #[case] output: Money,
    #[case] free: bool,
    #[case] log_fragment: Option<&str>,
) {
    let mut catalog = FakeCatalog::with_cache();
    explicit_free(&mut catalog, model);
    catalog.group(model, Some(input), Some(output));
    let (verdict, logs) = captured(|| is_model_cost_zero(&one(model), Some(&catalog)));
    assert_eq!(verdict, free, "the {model} rate classification");
    if let Some(fragment) = log_fragment {
        assert!(
            logs.iter().any(|line| line.contains(fragment)),
            "{model} logs {fragment}"
        );
    }
    assert_eq!(catalog.cached(model), Some(free));
}

#[test]
fn all_models_required() {
    let mut catalog = FakeCatalog::with_cache();
    explicit_free(&mut catalog, "free");
    catalog.group("paid", Some(money("0.5")), Some(money("0")));
    assert!(!is_model_cost_zero(
        &many(&["free", "paid"]),
        Some(&catalog)
    ));
    assert_eq!(catalog.cached("free"), Some(true));
    assert_eq!(catalog.cached("paid"), Some(false));
    assert!(is_model_cost_zero(&many(&["free", "free"]), Some(&catalog)));
}

#[test]
fn exception_not_cached() {
    let mut catalog = FakeCatalog::with_cache();
    catalog.group("broken", Some(money("0")), Some(money("0")));
    catalog.fail();
    let (verdict, logs) = captured(|| is_model_cost_zero(&one("broken"), Some(&catalog)));
    assert!(!verdict);
    assert!(logs.iter().any(|line| {
        line.starts_with("Error checking cost for model broken: the model catalog could not answer")
    }));
    assert_eq!(catalog.cached("broken"), None);
    assert_eq!(catalog.lookups(), vec!["group:broken".to_owned()]);
}

#[test]
fn safe_name_diagnostics() {
    let mut catalog = FakeCatalog::with_cache();
    explicit_free(&mut catalog, "free\nname\r1");
    explicit_free(&mut catalog, "ptu\nname");
    catalog.deployed(
        "ptu\nname",
        object(&[]),
        object(&[
            ("id", text("ptu\nname-capacity")),
            ("ptu_count", number("1")),
            ("cost_per_ptu_per_hour", number("2")),
        ]),
    );
    catalog.group("no\ncost", Some(money("0")), Some(money("0")));
    let (verdicts, logs) = captured(|| {
        (
            is_model_cost_zero(&one("free\nname\r1"), Some(&catalog)),
            is_model_cost_zero(&one("ptu\nname"), Some(&catalog)),
            is_model_cost_zero(&one("no\ncost"), Some(&catalog)),
        )
    });
    assert_eq!(verdicts, (true, false, false));
    assert!(
        logs.iter().any(|line| line
            == "Model freename1 has zero cost explicitly configured (input: 0, output: 0)")
    );
    assert!(
        logs.iter()
            .any(|line| line.starts_with("Model ptuname prices reserved PTU capacity"))
    );
    assert!(
        logs.iter()
            .any(|line| line.starts_with("Model nocost has zero cost but no explicit cost"))
    );
    assert!(
        !logs
            .iter()
            .any(|line| line.contains('\n') || line.contains('\r')),
        "the named diagnostics cannot forge a log line"
    );
}

#[test]
fn refresh_clears_cache() {
    let mut catalog = FakeCatalog::with_cache();
    explicit_free(&mut catalog, "free");
    assert!(is_model_cost_zero(&one("free"), Some(&catalog)));
    assert_eq!(catalog.cached("free"), Some(true));
    assert_eq!(
        catalog.cache.as_ref().map(ZeroCostCache::entries),
        Some(BTreeMap::from([("free".to_owned(), true)]))
    );
    catalog.group("free", Some(money("0.5")), Some(money("0")));
    assert!(
        is_model_cost_zero(&one("free"), Some(&catalog)),
        "the cached True still answers before the invalidation"
    );
    catalog.clear_cache();
    assert_eq!(
        catalog.cache.as_ref().map(ZeroCostCache::entries),
        Some(BTreeMap::new()),
        "the invalidation drops every verdict"
    );
    assert!(!is_model_cost_zero(&one("free"), Some(&catalog)));
    assert_eq!(catalog.cached("free"), Some(false));
}
