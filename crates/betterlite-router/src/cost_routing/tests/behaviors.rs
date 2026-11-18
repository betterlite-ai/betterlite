use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::support::{
    FakeWindows, GROUP, MINUTE, MINUTE_MILLIS, MODEL, NEXT_MINUTE, ProbeCosts, TestClock,
    deployment, entry, number, object, real_router, selection, spec, usage,
};
use crate::cost_routing::costs::DeploymentCosts;
use crate::cost_routing::service::{CostRouter, SuccessEvent};
use crate::deployment_identity::spec::DeploymentSpec;

const INPUT_COST: &str = "input_cost_per_token";
const OUTPUT_COST: &str = "output_cost_per_token";
const TPM: &str = "tpm";

#[derive(Clone, Copy)]
enum Behavior {
    RecordsBothCallbacks,
    SkipsBatchRetrieve,
    SkipsMissingMetadata,
    SeparateMinutes,
    FirstObservationOrder,
    PicksLowestCost,
    CustomPriceBeatsCatalog,
    SumsInputAndOutput,
    ZeroOverrideFallsBackToCatalog,
    UnknownModelCostsTen,
    NullCatalogPriceTakesFive,
    CatalogEntryFromUnpricedSibling,
    ScreensByCapacity,
    AppendsUnseenDeployments,
    KeepsDeploymentsGoneFromThePool,
    UnstoredGroupKeepsNothing,
    StoredOrderBeatsPoolOrder,
    RealCountersFeedSelection,
    SelectsNothingWhenEveryDeploymentIsFull,
}

#[rstest]
#[case::records_both_callbacks(Behavior::RecordsBothCallbacks)]
#[case::skips_batch_retrieve(Behavior::SkipsBatchRetrieve)]
#[case::skips_missing_metadata(Behavior::SkipsMissingMetadata)]
#[case::separate_minutes(Behavior::SeparateMinutes)]
#[case::first_observation_order(Behavior::FirstObservationOrder)]
#[case::picks_lowest_cost(Behavior::PicksLowestCost)]
#[case::custom_price_beats_catalog(Behavior::CustomPriceBeatsCatalog)]
#[case::sums_input_and_output(Behavior::SumsInputAndOutput)]
#[case::zero_override_falls_back_to_catalog(Behavior::ZeroOverrideFallsBackToCatalog)]
#[case::unknown_model_costs_ten(Behavior::UnknownModelCostsTen)]
#[case::null_catalog_price_takes_five(Behavior::NullCatalogPriceTakesFive)]
#[case::catalog_entry_from_unpriced_sibling(Behavior::CatalogEntryFromUnpricedSibling)]
#[case::screens_by_capacity(Behavior::ScreensByCapacity)]
#[case::appends_unseen_deployments(Behavior::AppendsUnseenDeployments)]
#[case::keeps_deployments_gone_from_the_pool(Behavior::KeepsDeploymentsGoneFromThePool)]
#[case::unstored_group_keeps_nothing(Behavior::UnstoredGroupKeepsNothing)]
#[case::stored_order_beats_pool_order(Behavior::StoredOrderBeatsPoolOrder)]
#[case::real_counters_feed_selection(Behavior::RealCountersFeedSelection)]
#[case::selects_nothing_when_full(Behavior::SelectsNothingWhenEveryDeploymentIsFull)]
#[tokio::test]
async fn cost_routing_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::RecordsBothCallbacks => records_both_callbacks().await,
        Behavior::SkipsBatchRetrieve => skips_batch_retrieve().await,
        Behavior::SkipsMissingMetadata => skips_missing_metadata().await,
        Behavior::SeparateMinutes => separate_minutes().await,
        Behavior::FirstObservationOrder => first_observation_order().await,
        Behavior::PicksLowestCost => picks_lowest_cost(),
        Behavior::CustomPriceBeatsCatalog => custom_price_beats_catalog(),
        Behavior::SumsInputAndOutput => sums_input_and_output(),
        Behavior::ZeroOverrideFallsBackToCatalog => zero_override_falls_back_to_catalog(),
        Behavior::UnknownModelCostsTen => unknown_model_costs_ten(),
        Behavior::NullCatalogPriceTakesFive => null_catalog_price_takes_five(),
        Behavior::CatalogEntryFromUnpricedSibling => catalog_entry_from_unpriced_sibling(),
        Behavior::ScreensByCapacity => screens_by_capacity(),
        Behavior::AppendsUnseenDeployments => appends_unseen_deployments().await,
        Behavior::KeepsDeploymentsGoneFromThePool => keeps_deployments_gone_from_the_pool().await,
        Behavior::UnstoredGroupKeepsNothing => unstored_group_keeps_nothing().await,
        Behavior::StoredOrderBeatsPoolOrder => stored_order_beats_pool_order().await,
        Behavior::RealCountersFeedSelection => real_counters_feed_selection().await,
        Behavior::SelectsNothingWhenEveryDeploymentIsFull => selects_nothing_when_full(),
    }
}

fn router_at(millis: u64) -> (CostRouter<FakeWindows>, TestClock) {
    let clock = TestClock::new(millis);
    (
        CostRouter::new(FakeWindows::default(), clock.io_clock()),
        clock,
    )
}

fn success(id: &str, tokens: i64) -> SuccessEvent<'_> {
    SuccessEvent {
        call_type: Some("acompletion"),
        model_group: Some(GROUP),
        deployment_id: Some(id),
        total_tokens: tokens,
    }
}

fn priced(id: &str, model: &str, input: &str, output: &str) -> DeploymentSpec {
    spec(
        id,
        model,
        &[(INPUT_COST, number(input)), (OUTPUT_COST, number(output))],
        &[],
        &[],
    )
}

fn catalog(model: &str, input: &str, output: &str) -> (&str, JsonObject) {
    (
        model,
        object(&[(INPUT_COST, number(input)), (OUTPUT_COST, number(output))]),
    )
}

// log_success_event and async_log_success_event are one primitive, so both call types land in the same window.
async fn records_both_callbacks() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let sync_only = SuccessEvent {
        call_type: None,
        ..success("a", 7)
    };
    router.record_success(&sync_only).await;
    router.record_success(&success("a", 3)).await;
    assert_eq!(router.entries(GROUP, MINUTE), vec![entry("a", 10, 2)]);
}

async fn skips_batch_retrieve() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    for call_type in ["aretrieve_batch", "retrieve_batch"] {
        let event = SuccessEvent {
            call_type: Some(call_type),
            ..success("a", 9)
        };
        router.record_success(&event).await;
    }
    assert!(router.entries(GROUP, MINUTE).is_empty());
}

async fn skips_missing_metadata() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    for event in [
        SuccessEvent {
            model_group: None,
            ..success("a", 9)
        },
        SuccessEvent {
            deployment_id: None,
            ..success("a", 9)
        },
    ] {
        router.record_success(&event).await;
    }
    assert!(router.entries(GROUP, MINUTE).is_empty());
}

async fn separate_minutes() {
    let (router, clock) = router_at(MINUTE_MILLIS);
    router.record_success(&success("a", 5)).await;
    clock.set_millis(MINUTE_MILLIS + 60_000);
    router.record_success(&success("a", 4)).await;
    assert_eq!(router.entries(GROUP, MINUTE), vec![entry("a", 5, 1)]);
    assert_eq!(router.entries(GROUP, NEXT_MINUTE), vec![entry("a", 4, 1)]);
}

async fn first_observation_order() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    router.record_success(&success("b", 1)).await;
    router.record_success(&success("a", 1)).await;
    assert_eq!(
        router.entries(GROUP, MINUTE),
        vec![entry("b", 1, 1), entry("a", 1, 1)]
    );
}

fn picks_lowest_cost() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let pool = [
        spec("a", "openai/a", &[], &[], &[]),
        spec("b", "openai/b", &[], &[], &[]),
    ];
    let costs = ProbeCosts::new(&[catalog("openai/a", "3", "1"), catalog("openai/b", "1", "1")]);
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(1)));
}

fn custom_price_beats_catalog() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let pool = [priced("a", MODEL, "0.01", "0.01"), deployment("b")];
    let costs = ProbeCosts::new(&[catalog(MODEL, "0.5", "0.5")]);
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(0)));
}

// item_cost is input_cost_per_token + output_cost_per_token, not a product with max_tokens or a single rate.
fn sums_input_and_output() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let pool = [
        spec("a", "openai/input", &[], &[], &[]),
        spec("b", "openai/output", &[], &[], &[]),
    ];
    let costs = ProbeCosts::new(&[
        catalog("openai/input", "0.0000001", "0.9"),
        catalog("openai/output", "0.1", "0.1"),
    ]);
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(1)));
}

// Python truthiness: a zero override is not a price, so the catalog entry fills both sides.
fn zero_override_falls_back_to_catalog() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let pool = [
        priced("a", "openai/zero", "0", "0"),
        spec("b", "openai/cheap", &[], &[], &[]),
    ];
    let costs = ProbeCosts::new(&[
        catalog("openai/zero", "0.3", "0.4"),
        catalog("openai/cheap", "0.1", "0.1"),
    ]);
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(1)));
}

// a model the cost map does not know is priced 5.0 + 5.0, the pinned fallback for each side.
fn unknown_model_costs_ten() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let pool = [
        spec("a", "openai/unknown", &[], &[], &[]),
        spec("b", "openai/known", &[], &[], &[]),
    ];
    let costs = ProbeCosts::new(&[catalog("openai/known", "0.01", "0.01")]);
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(1)));
}

// a null catalog rate takes the declared default rather than failing the route
fn null_catalog_price_takes_five() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let pool = [
        spec("a", "openai/null", &[], &[], &[]),
        spec("b", "openai/known", &[], &[], &[]),
    ];
    let nulls = object(&[(INPUT_COST, Json::Null), (OUTPUT_COST, number("0.1"))]);
    let costs = ProbeCosts::new(&[("openai/null", nulls), catalog("openai/known", "1", "1")]);
    // a null input rate is the 5.0 default, so 5.1 loses to 2.0
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(1)));
}

// get_model_info copies the catalog into model_info, and Deployment.__init__ mirrors custom prices over it.
fn catalog_entry_from_unpriced_sibling() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let zero = spec(
        "a",
        "openai/shared",
        &[(INPUT_COST, number("0")), (OUTPUT_COST, number("0"))],
        &[],
        &[],
    );
    let sibling = spec(
        "d",
        "openai/shared",
        &[],
        &[(INPUT_COST, number("0.3")), (OUTPUT_COST, number("0.4"))],
        &[],
    );
    let pool = [zero, spec("b", "openai/cheap", &[], &[], &[]), sibling];
    let costs = DeploymentCosts::from_deployments(&pool);
    // the zero override falls back to the sibling's catalog entry (0.7), so the cheap deployment wins
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(1)));
}

fn screens_by_capacity() {
    let windows = FakeWindows::default();
    windows.seed("a", MINUTE, usage(8, 0));
    let clock = TestClock::new(MINUTE_MILLIS);
    let router = CostRouter::new(windows, clock.io_clock());
    let pool = [
        spec(
            "a",
            MODEL,
            &[
                (INPUT_COST, number("0.01")),
                (OUTPUT_COST, number("0.01")),
                (TPM, number("10")),
            ],
            &[],
            &[],
        ),
        deployment("b"),
    ];
    let costs = ProbeCosts::default();
    // a would be the cheaper deployment, but 8 used + 5 estimated tokens pass its 10-token minute
    assert_eq!(router.select(&costs, &selection(&pool, 5)), Ok(Some(1)));
}

async fn appends_unseen_deployments() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    router.record_success(&success("a", 5)).await;
    let pool = [
        priced("a", MODEL, "0.1", "0.1"),
        priced("b", MODEL, "0.01", "0.01"),
    ];
    let costs = ProbeCosts::default();
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(1)));
    // the selector wrote b into the map with this minute's zero usage
    assert_eq!(
        router.entries(GROUP, MINUTE),
        vec![entry("a", 5, 1), entry("b", 0, 0)]
    );
}

async fn keeps_deployments_gone_from_the_pool() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    router.record_success(&success("gone", 5)).await;
    router.record_success(&success("a", 1)).await;
    let pool = [priced("a", MODEL, "0.5", "0.5")];
    let costs = ProbeCosts::new(&[catalog(MODEL, "0.1", "0.1")]);
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(0)));
    assert_eq!(
        router.entries(GROUP, MINUTE),
        vec![entry("gone", 5, 1), entry("a", 1, 1)]
    );
}

// The pinned map is stored by the first success, not by a selection: a fresh group's selection appends nothing.
async fn unstored_group_keeps_nothing() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    let pool = [
        priced("b", MODEL, "0.1", "0.1"),
        priced("a", MODEL, "0.1", "0.1"),
    ];
    let costs = ProbeCosts::default();
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(0)));
    assert!(router.entries(GROUP, MINUTE).is_empty());
    // the next selection rebuilds the map from the current pool, so the tie follows the new pool order
    let reordered = [
        priced("a", MODEL, "0.1", "0.1"),
        priced("b", MODEL, "0.1", "0.1"),
    ];
    assert_eq!(
        router.select(&costs, &selection(&reordered, 0)),
        Ok(Some(0))
    );
}

// Once a success stores the map, a deployment's first-observation place outranks the current pool order on a tie.
async fn stored_order_beats_pool_order() {
    let (router, _clock) = router_at(MINUTE_MILLIS);
    router.record_success(&success("b", 1)).await;
    let pool = [
        priced("a", MODEL, "0.1", "0.1"),
        priced("b", MODEL, "0.1", "0.1"),
    ];
    let costs = ProbeCosts::default();
    // the map order is b then a, so the equal-cost tie keeps b at pool index 1
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(1)));
    assert_eq!(
        router.entries(GROUP, MINUTE),
        vec![entry("b", 1, 1), entry("a", 0, 0)]
    );
}

async fn real_counters_feed_selection() {
    let clock = TestClock::new(MINUTE_MILLIS);
    let (router, _values) = real_router(&clock);
    router.record_success(&success("a", 60)).await;
    router.record_success(&success("b", 10)).await;
    assert_eq!(
        router.entries(GROUP, MINUTE),
        vec![entry("a", 60, 1), entry("b", 10, 1)]
    );
    let pool = [
        spec("a", MODEL, &[(TPM, number("100"))], &[], &[]),
        spec("b", MODEL, &[(TPM, number("100"))], &[], &[]),
    ];
    let costs = ProbeCosts::default();
    // 60 used + 50 estimated tokens pass a's 100-token minute, while b's 10 leave it eligible
    assert_eq!(router.select(&costs, &selection(&pool, 50)), Ok(Some(1)));
}

fn selects_nothing_when_full() {
    let windows = FakeWindows::default();
    windows.seed("a", MINUTE, usage(5, 0));
    windows.seed("b", MINUTE, usage(5, 0));
    let clock = TestClock::new(MINUTE_MILLIS);
    let router = CostRouter::new(windows, clock.io_clock());
    let pool = [
        spec("a", MODEL, &[(TPM, number("5"))], &[], &[]),
        spec("b", MODEL, &[(TPM, number("5"))], &[], &[]),
    ];
    let costs = ProbeCosts::new(&[catalog(MODEL, "0.1", "0.1")]);
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(None));
}
