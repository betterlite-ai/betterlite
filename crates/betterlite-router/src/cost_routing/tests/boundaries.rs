use std::time::{Duration, UNIX_EPOCH};

use betterlite_models::decisions::Json;
use rstest::rstest;

use super::support::{
    FakeWindows, GROUP, MINUTE, MINUTE_MILLIS, MODEL, NEXT_MINUTE, ProbeCosts, TestClock,
    deployment, entry, now, number, object, real_router, selection, spec, text, usage,
};
use crate::cost_routing::error::CostRoutingError;
use crate::cost_routing::keys::minute_label;
use crate::cost_routing::selection::Selection;
use crate::cost_routing::service::{
    CostRouter, MAX_DEPLOYMENTS_PER_GROUP, MAX_GROUPS, SuccessEvent,
};
use crate::cost_routing::window::CostWindows;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::routing_value_store::{RouteKey, RouteValue};

const INPUT_COST: &str = "input_cost_per_token";
const OUTPUT_COST: &str = "output_cost_per_token";
const TPM: &str = "tpm";
const RPM: &str = "rpm";

#[derive(Clone, Copy)]
enum Boundary {
    MinuteLabelAtKnownInstant,
    MinuteLabelBeforeEpoch,
    TpmEligibleAtExactLimit,
    TpmExcludedOneOver,
    EstimateAloneAtTheLimit,
    RpmEligibleAtExactLimit,
    RpmExcludedOneOver,
    MissingLimitsAreUnlimited,
    ZeroLimitIsUnlimited,
    EmptyContainersAreUnlimited,
    TopLevelLimitWins,
    LitellmParamsLimitWins,
    TieKeepsPoolOrder,
    TieKeepsFirstObservation,
    DuplicateIdLastInPoolWins,
    UnknownModelsTieAtTen,
    CustomOutputOnly,
    MissingCatalogFieldDefaults,
    NonNumericLimitNamesField,
    NonNumericPriceNamesField,
    OutOfRangePriceNamesField,
    BoolLimitIsRefused,
    RecordIsCancelledAtTheAwait,
    KeyAtLimit,
    KeyOverLimitFallsBackToZero,
    GroupCapDropsTheNewGroup,
    DeploymentCapDropsTheNewDeployment,
    WindowExpiresAfterTheTtl,
    EmptyPoolSelectsNone,
}

#[rstest]
#[case::minute_label_at_known_instant(Boundary::MinuteLabelAtKnownInstant)]
#[case::minute_label_before_epoch(Boundary::MinuteLabelBeforeEpoch)]
#[case::tpm_eligible_at_exact_limit(Boundary::TpmEligibleAtExactLimit)]
#[case::tpm_excluded_one_over(Boundary::TpmExcludedOneOver)]
#[case::estimate_alone_at_the_limit(Boundary::EstimateAloneAtTheLimit)]
#[case::rpm_eligible_at_exact_limit(Boundary::RpmEligibleAtExactLimit)]
#[case::rpm_excluded_one_over(Boundary::RpmExcludedOneOver)]
#[case::missing_limits_are_unlimited(Boundary::MissingLimitsAreUnlimited)]
#[case::zero_limit_is_unlimited(Boundary::ZeroLimitIsUnlimited)]
#[case::empty_containers_are_unlimited(Boundary::EmptyContainersAreUnlimited)]
#[case::top_level_limit_wins(Boundary::TopLevelLimitWins)]
#[case::litellm_params_limit_wins(Boundary::LitellmParamsLimitWins)]
#[case::tie_keeps_pool_order(Boundary::TieKeepsPoolOrder)]
#[case::tie_keeps_first_observation(Boundary::TieKeepsFirstObservation)]
#[case::duplicate_id_last_in_pool_wins(Boundary::DuplicateIdLastInPoolWins)]
#[case::unknown_models_tie_at_ten(Boundary::UnknownModelsTieAtTen)]
#[case::custom_output_only(Boundary::CustomOutputOnly)]
#[case::missing_catalog_field_defaults(Boundary::MissingCatalogFieldDefaults)]
#[case::non_numeric_limit_names_field(Boundary::NonNumericLimitNamesField)]
#[case::non_numeric_price_names_field(Boundary::NonNumericPriceNamesField)]
#[case::out_of_range_price_names_field(Boundary::OutOfRangePriceNamesField)]
#[case::bool_limit_is_refused(Boundary::BoolLimitIsRefused)]
#[case::record_is_cancelled_at_the_await(Boundary::RecordIsCancelledAtTheAwait)]
#[case::key_at_limit(Boundary::KeyAtLimit)]
#[case::key_over_limit_falls_back_to_zero(Boundary::KeyOverLimitFallsBackToZero)]
#[case::group_cap_drops_the_new_group(Boundary::GroupCapDropsTheNewGroup)]
#[case::deployment_cap_drops_the_new_deployment(Boundary::DeploymentCapDropsTheNewDeployment)]
#[case::window_expires_after_the_ttl(Boundary::WindowExpiresAfterTheTtl)]
#[case::empty_pool_selects_none(Boundary::EmptyPoolSelectsNone)]
#[tokio::test]
async fn cost_routing_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::MinuteLabelAtKnownInstant => minute_label_at_known_instant(),
        Boundary::MinuteLabelBeforeEpoch => minute_label_before_epoch(),
        Boundary::TpmEligibleAtExactLimit => tpm_eligible_at_exact_limit(),
        Boundary::TpmExcludedOneOver => tpm_excluded_one_over(),
        Boundary::EstimateAloneAtTheLimit => estimate_alone_at_the_limit(),
        Boundary::RpmEligibleAtExactLimit => rpm_eligible_at_exact_limit(),
        Boundary::RpmExcludedOneOver => rpm_excluded_one_over(),
        Boundary::MissingLimitsAreUnlimited => missing_limits_are_unlimited(),
        Boundary::ZeroLimitIsUnlimited => zero_limit_is_unlimited(),
        Boundary::EmptyContainersAreUnlimited => empty_containers_are_unlimited(),
        Boundary::TopLevelLimitWins => top_level_limit_wins(),
        Boundary::LitellmParamsLimitWins => litellm_params_limit_wins(),
        Boundary::TieKeepsPoolOrder => tie_keeps_pool_order(),
        Boundary::TieKeepsFirstObservation => tie_keeps_first_observation().await,
        Boundary::DuplicateIdLastInPoolWins => duplicate_id_last_in_pool_wins(),
        Boundary::UnknownModelsTieAtTen => unknown_models_tie_at_ten(),
        Boundary::CustomOutputOnly => custom_output_only(),
        Boundary::MissingCatalogFieldDefaults => missing_catalog_field_defaults(),
        Boundary::NonNumericLimitNamesField => non_numeric_limit_names_field(),
        Boundary::NonNumericPriceNamesField => non_numeric_price_names_field(),
        Boundary::OutOfRangePriceNamesField => out_of_range_price_names_field(),
        Boundary::BoolLimitIsRefused => bool_limit_is_refused(),
        Boundary::RecordIsCancelledAtTheAwait => record_is_cancelled_at_the_await().await,
        Boundary::KeyAtLimit => key_at_limit().await,
        Boundary::KeyOverLimitFallsBackToZero => key_over_limit_falls_back_to_zero().await,
        Boundary::GroupCapDropsTheNewGroup => group_cap_drops_the_new_group().await,
        Boundary::DeploymentCapDropsTheNewDeployment => {
            deployment_cap_drops_the_new_deployment().await;
        }
        Boundary::WindowExpiresAfterTheTtl => window_expires_after_the_ttl().await,
        Boundary::EmptyPoolSelectsNone => empty_pool_selects_none(),
    }
}

fn router_at(millis: u64) -> CostRouter<FakeWindows> {
    let clock = TestClock::new(millis);
    CostRouter::new(FakeWindows::default(), clock.io_clock())
}

fn seeded(seeds: &[(&str, i64, i64)]) -> CostRouter<FakeWindows> {
    let windows = FakeWindows::default();
    for &(id, tokens, requests) in seeds {
        windows.seed(id, MINUTE, usage(tokens, requests));
    }
    let clock = TestClock::new(MINUTE_MILLIS);
    CostRouter::new(windows, clock.io_clock())
}

fn success(id: &str, tokens: i64) -> SuccessEvent<'_> {
    SuccessEvent {
        call_type: None,
        model_group: Some(GROUP),
        deployment_id: Some(id),
        total_tokens: tokens,
    }
}

fn one(input: &str, output: &str) -> ProbeCosts {
    ProbeCosts::new(&[(
        MODEL,
        object(&[(INPUT_COST, number(input)), (OUTPUT_COST, number(output))]),
    )])
}

fn limited(limit: &str, value: &str) -> DeploymentSpec {
    spec("a", MODEL, &[(limit, number(value))], &[], &[])
}

fn rejected(costs: &ProbeCosts<'_>, pool: &[DeploymentSpec]) -> CostRoutingError {
    router_at(MINUTE_MILLIS)
        .select(costs, &selection(pool, 0))
        .unwrap_err()
}

// a deployment whose own field carries a value no price or limit conversion accepts
fn refused(field: &str, custom: Json) -> CostRoutingError {
    let pool = [spec("a", MODEL, &[(field, custom)], &[], &[])];
    rejected(&one("0.1", "0.1"), &pool)
}

fn minute_label_at_known_instant() {
    assert_eq!(minute_label(now(MINUTE_MILLIS)), MINUTE);
    assert_eq!(minute_label(now(MINUTE_MILLIS + 59_999)), MINUTE);
    assert_eq!(minute_label(now(MINUTE_MILLIS + 60_000)), NEXT_MINUTE);
}

fn minute_label_before_epoch() {
    let before_epoch = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();
    assert_eq!(minute_label(before_epoch), "1970-01-01-00-00");
}

fn tpm_eligible_at_exact_limit() {
    let router = seeded(&[("a", 100, 0)]);
    let pool = [limited(TPM, "100")];
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&pool, 0)),
        Ok(Some(0))
    );
}

fn tpm_excluded_one_over() {
    let pool = [limited(TPM, "100")];
    assert_eq!(
        seeded(&[("a", 100, 0)]).select(&one("0.1", "0.1"), &selection(&pool, 1)),
        Ok(None)
    );
}

fn estimate_alone_at_the_limit() {
    let router = router_at(MINUTE_MILLIS);
    let pool = [limited(TPM, "100")];
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&pool, 100)),
        Ok(Some(0))
    );
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&pool, 101)),
        Ok(None)
    );
}

fn rpm_eligible_at_exact_limit() {
    for requests in [0, 4] {
        let router = seeded(&[("a", 0, requests)]);
        let pool = [limited(RPM, "5")];
        assert_eq!(
            router.select(&one("0.1", "0.1"), &selection(&pool, 0)),
            Ok(Some(0))
        );
    }
}

fn rpm_excluded_one_over() {
    let pool = [limited(RPM, "5"), spec("b", MODEL, &[], &[], &[])];
    // a is one request past its rpm, so the deployment without a limit answers
    let picked = seeded(&[("a", 0, 5)]).select(&one("0.1", "0.1"), &selection(&pool, 0));
    assert_eq!(picked, Ok(Some(1)));
}

fn missing_limits_are_unlimited() {
    let router = seeded(&[("a", i64::MAX, i64::MAX)]);
    let pool = [deployment("a")];
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&pool, 0)),
        Ok(Some(0))
    );
}

// `or` chains read 0 as absent, so a top-level zero falls through and a lone model_info zero is infinity
fn zero_limit_is_unlimited() {
    let router = seeded(&[("a", 100, 0)]);
    let fall_through = [spec(
        "a",
        MODEL,
        &[(TPM, number("100"))],
        &[],
        &[(TPM, number("0"))],
    )];
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&fall_through, 0)),
        Ok(Some(0))
    );
    let zero_info = [spec("a", MODEL, &[], &[(TPM, number("0"))], &[])];
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&zero_info, 0)),
        Ok(Some(0))
    );
}

// Python truthiness reads an empty string, list or object as absent, so each limit source falls through
fn empty_containers_are_unlimited() {
    let router = seeded(&[("a", 100, 0)]);
    for empty in [text(""), Json::List(Vec::new()), Json::Object(object(&[]))] {
        let pool = [spec("a", MODEL, &[(TPM, empty)], &[], &[])];
        assert_eq!(
            router.select(&one("0.1", "0.1"), &selection(&pool, 0)),
            Ok(Some(0))
        );
    }
}

fn top_level_limit_wins() {
    let router = seeded(&[("a", 50, 0)]);
    let pool = [spec(
        "a",
        MODEL,
        &[(TPM, number("1"))],
        &[],
        &[(TPM, number("100"))],
    )];
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&pool, 0)),
        Ok(Some(0))
    );
}

fn litellm_params_limit_wins() {
    let router = seeded(&[("a", 50, 0)]);
    // the model_info limit of 1 would skip a; the params limit of 100 keeps it eligible
    let infos: [&[(&str, Json)]; 2] = [&[], &[(TPM, number("1"))]];
    for info in infos {
        let pool = [spec("a", MODEL, &[(TPM, number("100"))], info, &[])];
        assert_eq!(
            router.select(&one("0.1", "0.1"), &selection(&pool, 0)),
            Ok(Some(0))
        );
    }
}

fn tie_keeps_pool_order() {
    let router = router_at(MINUTE_MILLIS);
    let pool = [deployment("a"), deployment("b"), deployment("c")];
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&pool, 0)),
        Ok(Some(0))
    );
}

async fn tie_keeps_first_observation() {
    let router = router_at(MINUTE_MILLIS);
    router.record_success(&success("b", 1)).await;
    router.record_success(&success("a", 1)).await;
    let pool = [deployment("a"), deployment("b")];
    assert_eq!(
        router.select(&one("0.1", "0.1"), &selection(&pool, 0)),
        Ok(Some(1))
    );
}

// the pinned loop keeps the last healthy deployment with a matching id
fn duplicate_id_last_in_pool_wins() {
    let router = router_at(MINUTE_MILLIS);
    let pool = [
        spec(
            "dup",
            "openai/expensive",
            &[(INPUT_COST, number("5")), (OUTPUT_COST, number("5"))],
            &[],
            &[],
        ),
        spec(
            "dup",
            "openai/cheap",
            &[(INPUT_COST, number("0.01")), (OUTPUT_COST, number("0.01"))],
            &[],
            &[],
        ),
    ];
    assert_eq!(
        router.select(&ProbeCosts::default(), &selection(&pool, 0)),
        Ok(Some(1))
    );
}

fn unknown_models_tie_at_ten() {
    let router = router_at(MINUTE_MILLIS);
    let pool = [
        spec("a", "openai/one", &[], &[], &[]),
        spec("b", "openai/two", &[], &[], &[]),
    ];
    assert_eq!(
        router.select(&ProbeCosts::default(), &selection(&pool, 0)),
        Ok(Some(0))
    );
}

// an output override still leaves the input side to the catalog
fn custom_output_only() {
    let router = router_at(MINUTE_MILLIS);
    let pool = [
        spec("a", MODEL, &[(OUTPUT_COST, number("0.1"))], &[], &[]),
        spec("b", "openai/b", &[], &[], &[]),
    ];
    let costs = ProbeCosts::new(&[
        (
            MODEL,
            object(&[(INPUT_COST, number("0.1")), (OUTPUT_COST, number("0.9"))]),
        ),
        (
            "openai/b",
            object(&[(INPUT_COST, number("0.1")), (OUTPUT_COST, number("0.2"))]),
        ),
    ]);
    // a is 0.1 + 0.1 and b is 0.1 + 0.2; a lone 5.0 input default would put a at 5.1
    assert_eq!(router.select(&costs, &selection(&pool, 0)), Ok(Some(0)));
}

fn missing_catalog_field_defaults() {
    let router = router_at(MINUTE_MILLIS);
    let pool = [
        spec("a", "openai/i", &[], &[], &[]),
        spec("b", "openai/b", &[], &[], &[]),
    ];
    let input_only = ProbeCosts::new(&[
        ("openai/i", object(&[(INPUT_COST, number("1"))])),
        (
            "openai/b",
            object(&[(INPUT_COST, number("0.1")), (OUTPUT_COST, number("0.1"))]),
        ),
    ]);
    assert_eq!(
        router.select(&input_only, &selection(&pool, 0)),
        Ok(Some(1))
    );
    let output_only = ProbeCosts::new(&[
        ("openai/i", object(&[(OUTPUT_COST, number("1"))])),
        (
            "openai/b",
            object(&[(INPUT_COST, number("0.1")), (OUTPUT_COST, number("0.1"))]),
        ),
    ]);
    assert_eq!(
        router.select(&output_only, &selection(&pool, 0)),
        Ok(Some(1))
    );
}

fn non_numeric_limit_names_field() {
    let error = refused(TPM, text("10"));
    let message = "deployment a has a tpm that is not a number";
    assert_eq!(error.field(), TPM);
    assert_eq!(error.to_string(), message);
}

fn non_numeric_price_names_field() {
    for (field, custom) in [(INPUT_COST, text("cheap")), (OUTPUT_COST, Json::Bool(true))] {
        let error = refused(field, custom);
        assert_eq!(
            error,
            CostRoutingError::NonNumeric {
                deployment: "a".to_owned(),
                field,
            }
        );
    }
    let pool = [spec("a", "openai/bad", &[], &[], &[])];
    let costs = ProbeCosts::new(&[("openai/bad", object(&[(OUTPUT_COST, Json::Bool(true))]))]);
    let error = rejected(&costs, &pool);
    assert_eq!(error.field(), OUTPUT_COST);
}

// a price Decimal cannot hold fails the selection with the field it names; LiteLLM ranks the float
fn out_of_range_price_names_field() {
    let router = router_at(MINUTE_MILLIS);
    let huge = number("100000000000000000000000000000");
    let pool = [spec("a", MODEL, &[(INPUT_COST, huge)], &[], &[])];
    let error = router
        .select(&one("0.1", "0.1"), &selection(&pool, 0))
        .unwrap_err();
    assert_eq!(error.field(), INPUT_COST);
}

fn bool_limit_is_refused() {
    let error = refused(TPM, Json::Bool(true));
    assert_eq!(error.field(), TPM);
}

// the order entry is written before the window await, so a cancelled observation still names the deployment
async fn record_is_cancelled_at_the_await() {
    let clock = TestClock::new(MINUTE_MILLIS);
    let router = CostRouter::new(super::support::PendingWindows, clock.io_clock());
    let stopped = tokio::time::timeout(
        Duration::from_millis(10),
        router.record_success(&success("a", 1)),
    )
    .await;
    assert!(stopped.is_err());
    assert_eq!(router.entries(GROUP, MINUTE), vec![entry("a", 0, 0)]);
}

// cost_map:{group}:{id}:{minute}:tpm is 1024 bytes at the pair length below; the requests key is the same size
fn long_id_pair(total: usize) -> (String, String) {
    (format!("g{}", "x".repeat(total - 4)), "i".repeat(3))
}

fn long_event<'event>(group: &'event str, id: &'event str) -> SuccessEvent<'event> {
    SuccessEvent {
        call_type: None,
        model_group: Some(group),
        deployment_id: Some(id),
        total_tokens: 5,
    }
}

async fn key_at_limit() {
    let (group, id) = long_id_pair(993);
    let router = router_at(MINUTE_MILLIS);
    router.record_success(&long_event(&group, &id)).await;
    assert_eq!(router.entries(&group, MINUTE), vec![entry(&id, 5, 1)]);
}

async fn key_over_limit_falls_back_to_zero() {
    let (group, id) = long_id_pair(994);
    let router = router_at(MINUTE_MILLIS);
    // the write is dropped with a warning, and the deployment remains selectable with no usage
    router.record_success(&long_event(&group, &id)).await;
    assert_eq!(router.entries(&group, MINUTE), vec![entry(&id, 0, 0)]);
}

async fn group_cap_drops_the_new_group() {
    let router = router_at(MINUTE_MILLIS);
    for index in 0..MAX_GROUPS {
        let group = format!("g{index}");
        router.record_success(&long_event(&group, "a")).await;
    }
    router.record_success(&long_event("g-overflow", "a")).await;
    assert_eq!(router.entries("g0", MINUTE), vec![entry("a", 5, 1)]);
    assert!(router.entries("g-overflow", MINUTE).is_empty());
    // a selection over an untracked group still reads the pool's ids and screens them
    let pool = [deployment("a")];
    let request = Selection {
        group: "g-overflow",
        deployments: &pool,
        input_tokens: 0,
    };
    assert_eq!(router.select(&ProbeCosts::default(), &request), Ok(Some(0)));
}

async fn deployment_cap_drops_the_new_deployment() {
    let router = router_at(MINUTE_MILLIS);
    for index in 0..MAX_DEPLOYMENTS_PER_GROUP {
        let id = format!("d{index}");
        router.record_success(&success(&id, 1)).await;
    }
    router.record_success(&success("d-overflow", 1)).await;
    let mut pool: Vec<DeploymentSpec> = (0..MAX_DEPLOYMENTS_PER_GROUP)
        .map(|index| deployment(&format!("d{index}")))
        .collect();
    // a deployment the map cannot store is never appended, even when it would be the cheapest
    let mut cheapest = deployment("d-overflow");
    cheapest
        .litellm_params
        .insert(INPUT_COST.to_owned(), number("0.01"));
    cheapest
        .litellm_params
        .insert(OUTPUT_COST.to_owned(), number("0.01"));
    pool.push(cheapest);
    assert_eq!(
        router.select(&ProbeCosts::default(), &selection(&pool, 0)),
        Ok(Some(0))
    );
    let stored = router.entries(GROUP, MINUTE);
    assert_eq!(stored.len(), MAX_DEPLOYMENTS_PER_GROUP);
    assert_eq!(
        stored.first().map(|entry| entry.deployment_id.as_str()),
        Some("d0")
    );
    assert!(
        !stored
            .iter()
            .any(|found| found.deployment_id == "d-overflow")
    );
}

async fn window_expires_after_the_ttl() {
    let clock = TestClock::new(MINUTE_MILLIS);
    let (router, _values) = real_router(&clock);
    router.record_success(&success("a", 5)).await;
    assert_eq!(router.entries(GROUP, MINUTE), vec![entry("a", 5, 1)]);
    clock.set_millis(MINUTE_MILLIS + 60_000);
    // the stored expiry is inclusive for one more millisecond
    assert_eq!(router.entries(GROUP, MINUTE), vec![entry("a", 5, 1)]);
    clock.set_millis(MINUTE_MILLIS + 60_001);
    assert_eq!(router.entries(GROUP, MINUTE), vec![entry("a", 0, 0)]);
}

fn empty_pool_selects_none() {
    let router = router_at(MINUTE_MILLIS);
    let pool: [DeploymentSpec; 0] = [];
    assert_eq!(
        router.select(&ProbeCosts::default(), &selection(&pool, 0)),
        Ok(None)
    );
}
