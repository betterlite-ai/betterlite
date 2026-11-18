use std::time::Duration;

use betterlite_config::sections::tier_artifact_predictor::ArtifactSource;
use betterlite_config::sections::tier_artifact_predictor::artifact::{
    Artifact, CohortStatistic, DomainStatistic, GlobalStatistic,
};
use rstest::rstest;
use tokio::time::timeout;

use super::{artifact_json, parsed, predictor, statistics};
use crate::adaptive_classification::request_type::RequestType;
use crate::tier_artifact_predictor::artifact::{
    MAX_ARTIFACT_BYTES, TierNumber, parse_artifact, resolve_tier_artifact,
};
use crate::tier_artifact_predictor::error::PredictorError;
use crate::tier_artifact_predictor::predictor::TierSuccessPredictor;
use crate::tier_artifact_predictor::validation;

#[derive(Clone, Copy)]
enum Boundary {
    ArtifactBytesAtLimit,
    ArtifactBytesOverLimit,
    MalformedJson,
    UnknownArtifact,
    SchemaVersion,
    TierZero,
    TierFive,
    SuccessesEqualObservations,
    SuccessesExceedObservations,
    ZeroObservations,
    NegativeSuccesses,
    NonFiniteSuccesses,
    NonFiniteObservations,
    NonFinitePriorMass,
    ZeroPriorMass,
    ThresholdOutside {
        threshold: f64,
    },
    DuplicateGlobalStatistic,
    MissingGlobalStatistic,
    DuplicateDomainStatistic,
    DuplicateCohortStatistic,
    UnknownRequestType,
    EmptyCohort,
    EmptyDatasetField {
        fields: &'static str,
        expected: &'static str,
    },
    EmptyArtifactField {
        field: &'static str,
    },
    EmptyDatasetRows,
    DatasetRowsAboveU32,
    ThresholdSelection {
        threshold: Option<f64>,
        expected: TierNumber,
    },
    CompletesWithoutAwaiting,
    CancelledForecastLeavesNoState,
    DomainPosteriorOverflow,
    CohortPosteriorOverflow,
}

#[rstest]
#[case::artifact_bytes_at_limit(Boundary::ArtifactBytesAtLimit)]
#[case::artifact_bytes_over_limit(Boundary::ArtifactBytesOverLimit)]
#[case::malformed_json(Boundary::MalformedJson)]
#[case::unknown_artifact(Boundary::UnknownArtifact)]
#[case::schema_version(Boundary::SchemaVersion)]
#[case::tier_zero(Boundary::TierZero)]
#[case::tier_five(Boundary::TierFive)]
#[case::successes_equal_observations(Boundary::SuccessesEqualObservations)]
#[case::successes_exceed_observations(Boundary::SuccessesExceedObservations)]
#[case::zero_observations(Boundary::ZeroObservations)]
#[case::negative_successes(Boundary::NegativeSuccesses)]
#[case::non_finite_successes(Boundary::NonFiniteSuccesses)]
#[case::non_finite_observations(Boundary::NonFiniteObservations)]
#[case::non_finite_prior_mass(Boundary::NonFinitePriorMass)]
#[case::zero_prior_mass(Boundary::ZeroPriorMass)]
#[case::threshold_above_one(Boundary::ThresholdOutside { threshold: 1.5_f64 })]
#[case::threshold_below_zero(Boundary::ThresholdOutside { threshold: -0.01_f64 })]
#[case::threshold_nan(Boundary::ThresholdOutside {
    threshold: f64::NAN
})]
#[case::duplicate_global_statistic(Boundary::DuplicateGlobalStatistic)]
#[case::missing_global_statistic(Boundary::MissingGlobalStatistic)]
#[case::duplicate_domain_statistic(Boundary::DuplicateDomainStatistic)]
#[case::duplicate_cohort_statistic(Boundary::DuplicateCohortStatistic)]
#[case::unknown_request_type(Boundary::UnknownRequestType)]
#[case::empty_cohort(Boundary::EmptyCohort)]
#[case::empty_dataset_name(Boundary::EmptyDatasetField {
    fields: "\"datasets\": [{\"name\": \"\", \"url\": \"u\", \"license\": \"MIT\", \"rows\": 1}]",
    expected: "datasets[0].name",
})]
#[case::empty_dataset_url(Boundary::EmptyDatasetField {
    fields: "\"datasets\": [{\"name\": \"n\", \"url\": \"\", \"license\": \"MIT\", \"rows\": 1}]",
    expected: "datasets[0].url",
})]
#[case::empty_dataset_license(Boundary::EmptyDatasetField {
    fields: "\"datasets\": [{\"name\": \"n\", \"url\": \"u\", \"license\": \"\", \"rows\": 1}]",
    expected: "datasets[0].license",
})]
#[case::empty_dataset_success_definition(Boundary::EmptyDatasetField {
    fields: "\"datasets\": [{\"name\": \"n\", \"url\": \"u\", \"license\": \"MIT\", \"rows\": 1, \"success_definition\": \"\"}]",
    expected: "datasets[0].success_definition",
})]
#[case::empty_success_definition(Boundary::EmptyArtifactField {
    field: "success_definition"
})]
#[case::empty_split_method(Boundary::EmptyArtifactField { field: "split_method" })]
#[case::empty_dataset_rows(Boundary::EmptyDatasetRows)]
#[case::dataset_rows_above_u32(Boundary::DatasetRowsAboveU32)]
#[case::threshold_at_probability(Boundary::ThresholdSelection {
    threshold: None,
    expected: TierNumber::One,
})]
// one unit in the last place above 0.75 no tier reaches; one below keeps the first tier
#[case::threshold_one_ulp_above(Boundary::ThresholdSelection {
    threshold: Some(0.750_000_000_000_000_1_f64),
    expected: TierNumber::Four,
})]
#[case::threshold_one_ulp_below(Boundary::ThresholdSelection {
    threshold: Some(0.749_999_999_999_999_9_f64),
    expected: TierNumber::One,
})]
#[case::threshold_zero(Boundary::ThresholdSelection {
    threshold: Some(0.0_f64),
    expected: TierNumber::One,
})]
#[case::threshold_one(Boundary::ThresholdSelection {
    threshold: Some(1.0_f64),
    expected: TierNumber::Four,
})]
#[case::completes_without_awaiting(Boundary::CompletesWithoutAwaiting)]
#[case::cancelled_forecast_leaves_no_state(Boundary::CancelledForecastLeavesNoState)]
#[case::domain_posterior_overflow(Boundary::DomainPosteriorOverflow)]
#[case::cohort_posterior_overflow(Boundary::CohortPosteriorOverflow)]
#[tokio::test]
async fn tier_artifact_predictor_boundaries(#[case] boundary: Boundary) {
    match boundary {
        Boundary::ArtifactBytesAtLimit => artifact_bytes_at_limit(),
        Boundary::ArtifactBytesOverLimit => artifact_bytes_over_limit(),
        Boundary::MalformedJson => malformed_json(),
        Boundary::UnknownArtifact => unknown_artifact(),
        Boundary::SchemaVersion => schema_version(),
        Boundary::TierZero => tier_out_of_range(0),
        Boundary::TierFive => tier_out_of_range(5),
        Boundary::SuccessesEqualObservations => successes_equal_observations(),
        Boundary::SuccessesExceedObservations => successes_exceed_observations(),
        Boundary::ZeroObservations => zero_observations(),
        Boundary::NegativeSuccesses => negative_successes(),
        Boundary::NonFiniteSuccesses => non_finite_successes(),
        Boundary::NonFiniteObservations => non_finite_observations(),
        Boundary::NonFinitePriorMass => non_finite_prior_mass(),
        Boundary::ZeroPriorMass => zero_prior_mass(),
        Boundary::ThresholdOutside { threshold } => threshold_outside(threshold),
        Boundary::DuplicateGlobalStatistic => duplicate_global_statistic(),
        Boundary::MissingGlobalStatistic => missing_global_statistic(),
        Boundary::DuplicateDomainStatistic => duplicate_domain_statistic(),
        Boundary::DuplicateCohortStatistic => duplicate_cohort_statistic(),
        Boundary::UnknownRequestType => unknown_request_type(),
        Boundary::EmptyCohort => empty_cohort(),
        Boundary::EmptyDatasetField { fields, expected } => {
            empty_field(fields, expected);
        }
        Boundary::EmptyArtifactField { field } => empty_artifact_field(field),
        Boundary::EmptyDatasetRows => empty_dataset_rows(),
        Boundary::DatasetRowsAboveU32 => dataset_rows_above_u32(),
        Boundary::ThresholdSelection {
            threshold,
            expected,
        } => threshold_selection(threshold, expected),
        Boundary::CompletesWithoutAwaiting => completes_without_awaiting().await,
        Boundary::CancelledForecastLeavesNoState => cancelled_forecast_leaves_no_state(),
        // near f64::MAX a huge prior overflows the posterior sums into inf/inf, which is NaN
        Boundary::DomainPosteriorOverflow => {
            let mut artifact = draft([1e308_f64; 4], [1e308_f64; 4], 0.75);
            artifact.domain_prior_mass = 1.7e308_f64;
            artifact.domain_statistics = vec![DomainStatistic {
                tier: 1,
                successes: 1e308,
                observations: 1e308,
                request_type: "general".to_owned(),
            }];
            let statistic = non_finite_posterior(&artifact);
            assert_eq!(statistic, "the domain statistic for general at tier SIMPLE");
        }
        Boundary::CohortPosteriorOverflow => {
            let mut artifact = draft([1.0_f64; 4], [2.0_f64; 4], 0.75);
            artifact.cohort_prior_mass = 1.7e308_f64;
            artifact.domain_statistics = vec![DomainStatistic {
                tier: 1,
                successes: 1.0,
                observations: 2.0,
                request_type: "general".to_owned(),
            }];
            artifact.cohort_statistics = vec![CohortStatistic {
                tier: 1,
                successes: 1e308,
                observations: 1e308,
                cohort: "general|short|code=0|math=0|mc=0|intl=0".to_owned(),
            }];
            let statistic = non_finite_posterior(&artifact);
            assert_eq!(
                statistic,
                "the cohort statistic general|short|code=0|math=0|mc=0|intl=0 at tier SIMPLE"
            );
        }
    }
}

fn draft(successes: [f64; 4], observations: [f64; 4], routing_threshold: f64) -> Artifact {
    let global_statistics = [1_u32, 2, 3, 4]
        .into_iter()
        .enumerate()
        .map(|(index, tier)| GlobalStatistic {
            tier,
            successes: successes[index],
            observations: observations[index],
        })
        .collect();
    Artifact {
        schema_version: 1,
        global_statistics,
        domain_statistics: Vec::new(),
        cohort_statistics: Vec::new(),
        domain_prior_mass: 200.0,
        cohort_prior_mass: 20.0,
        routing_threshold,
        datasets: Vec::new(),
        success_definition: "quality score meets the dataset success threshold".to_owned(),
        split_method: "sha256(prompt): 70% train, 15% validation, 15% test".to_owned(),
    }
}

fn valid_bytes() -> Vec<u8> {
    artifact_json(
        &super::global_json([(1.0, 2.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0)]),
        "",
    )
    .into_bytes()
}

fn artifact_bytes_at_limit() {
    let mut bytes = valid_bytes();
    bytes.resize(MAX_ARTIFACT_BYTES, b' ');
    let artifact = parse_artifact(&bytes).expect("the bound is inclusive");
    assert_eq!(artifact.routing_threshold.to_bits(), 0.75_f64.to_bits());
}

fn artifact_bytes_over_limit() {
    let mut bytes = valid_bytes();
    bytes.resize(MAX_ARTIFACT_BYTES.saturating_add(1), b' ');
    match parse_artifact(&bytes) {
        Err(PredictorError::ArtifactTooLarge {
            bytes: length,
            limit,
        }) => {
            assert_eq!(length, MAX_ARTIFACT_BYTES.saturating_add(1));
            assert_eq!(limit, MAX_ARTIFACT_BYTES);
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

fn malformed_json() {
    match parse_artifact(b"{\"global_statistics\": [") {
        Err(PredictorError::ArtifactJson { source }) => {
            assert!(!source.to_string().is_empty());
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

fn unknown_artifact() {
    match resolve_tier_artifact(&ArtifactSource::BuiltIn("arc_agi".to_owned())) {
        Err(PredictorError::UnknownArtifact { name }) => assert_eq!(name, "arc_agi"),
        other => panic!("unexpected outcome: {other:?}"),
    }
}

fn parse_error(global: &str, fields: &str) -> PredictorError {
    parse_artifact(artifact_json(global, fields).as_bytes())
        .expect_err("the crafted artifact is refused")
}

fn schema_version() {
    let global = super::global_json([
        (1.0_f64, 2.0_f64),
        (2.0_f64, 6.0_f64),
        (1.0_f64, 2.0_f64),
        (4.0_f64, 6.0_f64),
    ]);
    let fields = ", \"schema_version\": 2";
    let error = parse_error(&global, fields);
    let PredictorError::SchemaVersion { version } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(version, 2);
}

fn tier_out_of_range(tier: u32) {
    let statistic = format!("{{\"tier\": {tier}, \"successes\": 0.0, \"observations\": 1.0}}");
    let error = parse_error(&statistic, "");
    let PredictorError::TierRange { tier: declared } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(declared, tier);
}

fn successes_equal_observations() {
    let artifact = parsed(
        &super::global_json([
            (1.0_f64, 1.0_f64),
            (2.0_f64, 2.0_f64),
            (1.0_f64, 1.0_f64),
            (0.0_f64, 1.0_f64),
        ]),
        "",
    );
    assert_eq!(artifact.global.one.successes.to_bits(), 1.0_f64.to_bits());
    assert_eq!(
        artifact.global.one.observations.to_bits(),
        1.0_f64.to_bits()
    );
}

fn successes_exceed_observations() {
    let statistic = "{\"tier\": 1, \"successes\": 2.0, \"observations\": 1.0}";
    let error = parse_error(statistic, "");
    let PredictorError::SuccessesExceedObservations {
        field,
        successes,
        observations,
    } = error
    else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(field, "global_statistics[0]");
    assert_eq!(successes.to_bits(), 2.0_f64.to_bits());
    assert_eq!(observations.to_bits(), 1.0_f64.to_bits());
}

fn zero_observations() {
    let statistic = "{\"tier\": 1, \"successes\": 0.0, \"observations\": 0.0}";
    let error = parse_error(statistic, "");
    let PredictorError::NonPositiveObservations { field, value } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(field, "global_statistics[0].observations");
    assert_eq!(value.to_bits(), 0.0_f64.to_bits());
}

fn negative_successes() {
    let statistic = "{\"tier\": 1, \"successes\": -0.5, \"observations\": 1.0}";
    let error = parse_error(statistic, "");
    let PredictorError::NegativeSuccesses { field, value } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(field, "global_statistics[0].successes");
    assert_eq!(value.to_bits(), (-0.5_f64).to_bits());
}

fn non_finite_successes() {
    let mut artifact = draft([1.0_f64; 4], [2.0_f64; 4], 0.75);
    artifact.global_statistics[0].successes = f64::NAN;
    assert_eq!(rejected_field(&artifact), "global_statistics[0].successes");
    artifact.global_statistics[0].successes = f64::INFINITY;
    assert_eq!(rejected_field(&artifact), "global_statistics[0].successes");
}

fn non_finite_observations() {
    let mut artifact = draft([1.0_f64; 4], [2.0_f64; 4], 0.75);
    artifact.global_statistics[3].observations = f64::INFINITY;
    assert_eq!(
        rejected_field(&artifact),
        "global_statistics[3].observations"
    );
    artifact.global_statistics[3].observations = f64::NAN;
    assert_eq!(
        rejected_field(&artifact),
        "global_statistics[3].observations"
    );
}

fn rejected_field(artifact: &Artifact) -> String {
    match validation::validate(artifact) {
        Err(PredictorError::NonFinite { field }) => field,
        other => panic!("unexpected outcome: {other:?}"),
    }
}

fn non_finite_prior_mass() {
    let mut artifact = draft([1.0_f64; 4], [2.0_f64; 4], 0.75);
    artifact.domain_prior_mass = f64::INFINITY;
    match validation::validate(&artifact) {
        Err(PredictorError::NonFinite { field }) => assert_eq!(field, "domain_prior_mass"),
        other => panic!("unexpected outcome: {other:?}"),
    }
    artifact.domain_prior_mass = 200.0_f64;
    artifact.cohort_prior_mass = f64::NAN;
    match validation::validate(&artifact) {
        Err(PredictorError::NonFinite { field }) => assert_eq!(field, "cohort_prior_mass"),
        other => panic!("unexpected outcome: {other:?}"),
    }
}

fn zero_prior_mass() {
    let mut artifact = draft([1.0_f64; 4], [2.0_f64; 4], 0.75);
    artifact.domain_prior_mass = 0.0_f64;
    match validation::validate(&artifact) {
        Err(PredictorError::PriorMass { field, mass }) => {
            assert_eq!(field, "domain_prior_mass");
            assert_eq!(mass.to_bits(), 0.0_f64.to_bits());
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    artifact.domain_prior_mass = 200.0_f64;
    artifact.cohort_prior_mass = -1.0_f64;
    assert!(matches!(
        validation::validate(&artifact),
        Err(PredictorError::PriorMass { .. })
    ));
}

fn threshold_outside(threshold: f64) {
    let mut artifact = draft([1.0_f64; 4], [2.0_f64; 4], threshold);
    match validation::validate(&artifact) {
        Err(PredictorError::ThresholdRange { threshold: got }) => {
            assert_eq!(got.to_bits(), threshold.to_bits());
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
    artifact.routing_threshold = 0.0_f64;
    validation::validate(&artifact).unwrap();
    artifact.routing_threshold = 1.0_f64;
    validation::validate(&artifact).unwrap();
}

fn duplicate_global_statistic() {
    let global = "{\"tier\": 1, \"successes\": 1.0, \"observations\": 2.0}, {\"tier\": 1, \"successes\": 1.0, \"observations\": 2.0}, {\"tier\": 2, \"successes\": 1.0, \"observations\": 2.0}, {\"tier\": 3, \"successes\": 1.0, \"observations\": 2.0}";
    let error = parse_error(global, "");
    // the error names the repeated statistic and renders the pinned upper-case tier name
    assert_eq!(
        error.to_string(),
        "the tier artifact declares statistic global_statistics[1] twice for tier SIMPLE"
    );
    let PredictorError::DuplicateStatistic { field, tier } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(field, "global_statistics[1]");
    assert_eq!(tier, "SIMPLE");
}

fn missing_global_statistic() {
    let global = "{\"tier\": 1, \"successes\": 1.0, \"observations\": 2.0}, {\"tier\": 2, \"successes\": 1.0, \"observations\": 2.0}, {\"tier\": 3, \"successes\": 1.0, \"observations\": 2.0}";
    assert!(matches!(
        parse_error(global, ""),
        PredictorError::MissingGlobalStatistics
    ));
    assert!(matches!(
        parse_error("", ""),
        PredictorError::MissingGlobalStatistics
    ));
}

fn four_global() -> String {
    super::global_json([(1.0, 2.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0)])
}

fn duplicate_domain_statistic() {
    let fields = ", \"domain_statistics\": [{\"tier\": 2, \"successes\": 1.0, \"observations\": 2.0, \"request_type\": \"general\"}, {\"tier\": 2, \"successes\": 1.0, \"observations\": 2.0, \"request_type\": \"general\"}]";
    let error = parse_error(&four_global(), fields);
    let PredictorError::DuplicateStatistic { field, tier } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(field, "domain_statistics[1]");
    assert_eq!(tier, "MEDIUM");
}

fn duplicate_cohort_statistic() {
    let fields = ", \"cohort_statistics\": [{\"tier\": 3, \"successes\": 1.0, \"observations\": 2.0, \"cohort\": \"general|short|code=0|math=0|mc=0|intl=0\"}, {\"tier\": 3, \"successes\": 1.0, \"observations\": 2.0, \"cohort\": \"general|short|code=0|math=0|mc=0|intl=0\"}]";
    let text = parse_error(&four_global(), fields).to_string();
    assert!(
        text.contains("statistic cohort_statistics[1] twice"),
        "got {text}"
    );
    assert!(text.ends_with("twice for tier COMPLEX"), "got {text}");
}

fn unknown_request_type() {
    let fields = ", \"domain_statistics\": [{\"tier\": 1, \"successes\": 1.0, \"observations\": 2.0, \"request_type\": \"billing\"}]";
    let error = parse_error(&four_global(), fields);
    let PredictorError::UnknownRequestType { request_type } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(request_type, "billing");
}

fn empty_cohort() {
    let fields = ", \"cohort_statistics\": [{\"tier\": 1, \"successes\": 1.0, \"observations\": 2.0, \"cohort\": \"\"}]";
    let error = parse_error(&four_global(), fields);
    let PredictorError::EmptyField { field } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(field, "cohort_statistics[0].cohort");
}

fn empty_field(fields: &str, expected: &str) {
    let error = parse_error(&four_global(), &format!(", {fields}"));
    let PredictorError::EmptyField { field } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(field, expected);
}

fn empty_artifact_field(field: &str) {
    let fields = format!(", \"{field}\": \"\"");
    let error = parse_error(&four_global(), &fields);
    let PredictorError::EmptyField { field: got } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(got, field);
}

fn empty_dataset_rows() {
    let fields =
        ", \"datasets\": [{\"name\": \"n\", \"url\": \"u\", \"license\": \"MIT\", \"rows\": 0}]";
    let error = parse_error(&four_global(), fields);
    let PredictorError::EmptyDatasetRows { index } = error else {
        panic!("unexpected error: {error:?}");
    };
    assert_eq!(index, 0);
}

// rows stay a u32, the width the packaged attribution fits, where the reference declares an int
fn dataset_rows_above_u32() {
    let fields = ", \"datasets\": [{\"name\": \"n\", \"url\": \"u\", \"license\": \"MIT\", \"rows\": 4294967296}]";
    let error = parse_error(&four_global(), fields);
    let PredictorError::ArtifactJson { .. } = error else {
        panic!("unexpected error: {error:?}");
    };
}

fn threshold_selection(threshold: Option<f64>, expected: TierNumber) {
    let artifact = super::artifact_with(statistics(
        (2.0_f64, 2.0_f64),
        (0.0_f64, 8.0_f64),
        (0.0_f64, 8.0_f64),
        (0.0_f64, 8.0_f64),
    ));
    let predictor =
        TierSuccessPredictor::new(artifact, threshold).expect("the pinned patterns compile");
    let prediction = predictor.predict("Hello.", RequestType::General);
    assert_eq!(
        prediction.probabilities.map(f64::to_bits),
        [0.75_f64.to_bits(); 4]
    );
    assert_eq!(prediction.required_tier, expected);
}

// the predictor is synchronous and pure, so a prediction finishes without yielding to the runtime
async fn completes_without_awaiting() {
    let predictor = predictor(statistics(
        (0.0_f64, 6.0_f64),
        (2.0_f64, 6.0_f64),
        (1.0_f64, 2.0_f64),
        (4.0_f64, 6.0_f64),
    ));
    let completed = timeout(Duration::from_millis(100), async move {
        predictor.predict("Hello.", RequestType::General)
    })
    .await;
    let prediction = completed.expect("the synchronous prediction completes without awaiting");
    assert_eq!(
        prediction.probabilities.map(f64::to_bits),
        [
            0.125_f64.to_bits(),
            0.375_f64.to_bits(),
            0.5_f64.to_bits(),
            0.625_f64.to_bits(),
        ]
    );
}

fn cancelled_forecast_leaves_no_state() {
    let predictor = predictor(statistics(
        (0.0_f64, 6.0_f64),
        (2.0_f64, 6.0_f64),
        (1.0_f64, 2.0_f64),
        (4.0_f64, 6.0_f64),
    ));
    let _cancelled = predictor.predict("Hello.", RequestType::General);
    let prediction = predictor.predict("Hello.", RequestType::General);
    assert_eq!(prediction.required_tier, TierNumber::Four);
}

fn non_finite_posterior(artifact: &Artifact) -> String {
    match validation::validate(artifact) {
        Err(PredictorError::NonFinitePosterior { statistic }) => statistic,
        other => panic!("unexpected outcome: {other:?}"),
    }
}
