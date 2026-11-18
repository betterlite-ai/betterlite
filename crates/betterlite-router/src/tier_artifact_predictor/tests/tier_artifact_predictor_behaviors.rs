use betterlite_config::sections::tier_artifact_predictor::ArtifactSource;
use rstest::rstest;

use super::{
    CONFIG_SEED, artifact_with, corpus_config, global_json, parsed, predictor, statistics, texts,
};
use crate::adaptive_classification::request_type::RequestType;
use crate::complexity_scoring::decision::RoutingDecisionCause;
use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::tier_artifact_predictor::artifact::{
    GlobalStatistics, TierNumber, TierStatistic, resolve_tier_artifact,
};
use crate::tier_artifact_predictor::cohort::CohortMatcher;
use crate::tier_artifact_predictor::predictor::{TierArtifactPredictor, TierSuccessPredictor};

#[derive(Clone, Copy)]
enum Behavior {
    GlobalOnly,
    DomainPosterior,
    CohortPosterior,
    Monotone,
    ThresholdDefault,
    ThresholdOverride,
    FirstTierAtThreshold,
    LastTierFallback,
    Signals,
    ClassifiedTier,
    Forecast,
    ForecastPrecision,
    BuiltInArtifact,
    Attribution,
    ShippedConfigSeed,
    CohortFlags {
        prompt: &'static str,
        request_type: RequestType,
        expected: &'static str,
    },
    CohortBucket {
        length: usize,
        expected: &'static str,
    },
    UnicodeSelection {
        prompt: &'static str,
        expected: TierNumber,
    },
    EmptyPrompt,
}

#[rstest]
#[case::global_only(Behavior::GlobalOnly)]
#[case::domain_posterior(Behavior::DomainPosterior)]
#[case::cohort_posterior(Behavior::CohortPosterior)]
#[case::monotone(Behavior::Monotone)]
#[case::threshold_default(Behavior::ThresholdDefault)]
#[case::threshold_override(Behavior::ThresholdOverride)]
#[case::first_tier_at_threshold(Behavior::FirstTierAtThreshold)]
#[case::last_tier_fallback(Behavior::LastTierFallback)]
#[case::signals(Behavior::Signals)]
#[case::classified_tier(Behavior::ClassifiedTier)]
#[case::forecast(Behavior::Forecast)]
#[case::forecast_precision(Behavior::ForecastPrecision)]
#[case::builtin_artifact(Behavior::BuiltInArtifact)]
#[case::attribution(Behavior::Attribution)]
#[case::shipped_config_seed(Behavior::ShippedConfigSeed)]
#[case::code_flags(Behavior::CohortFlags {
    prompt: "Write a Python function that adds two numbers.",
    request_type: RequestType::General,
    expected: "general|short|code=1|math=0|mc=0|intl=0",
})]
#[case::math_flag(Behavior::CohortFlags {
    prompt: "Solve the equation 2x + 3 = 7.",
    request_type: RequestType::AnalyticalReasoning,
    expected: "analytical_reasoning|short|code=0|math=1|mc=0|intl=0",
})]
#[case::multiple_choice_flag(Behavior::CohortFlags {
    prompt: "Which outcome is right? A. first option B. second option",
    request_type: RequestType::General,
    expected: "general|short|code=0|math=0|mc=1|intl=0",
})]
#[case::unicode_flag(Behavior::CohortFlags {
    prompt: "Привет, как дела? Расскажи о себе.",
    request_type: RequestType::General,
    expected: "general|short|code=0|math=0|mc=0|intl=1",
})]
// one code point in ten is exactly the 0.1 the reference requires to exceed
#[case::ratio_at_threshold(Behavior::CohortFlags {
    prompt: "éabcdefghi",
    request_type: RequestType::General,
    expected: "general|short|code=0|math=0|mc=0|intl=0",
})]
#[case::ratio_above_threshold(Behavior::CohortFlags {
    prompt: "ééabcdefgh",
    request_type: RequestType::General,
    expected: "general|short|code=0|math=0|mc=0|intl=1",
})]
// U+0301 is not a word character for Python's \b, so the keyword "code" still ends there
#[case::combining_mark_after_code(Behavior::CohortFlags {
    prompt: "code\u{301}",
    request_type: RequestType::General,
    expected: "general|short|code=1|math=0|mc=0|intl=1",
})]
// re.IGNORECASE folds the dotless i to i, so "function" matches with U+0131 in place of i
#[case::dotless_i_in_function(Behavior::CohortFlags {
    prompt: "funct\u{131}on",
    request_type: RequestType::General,
    expected: "general|short|code=1|math=0|mc=0|intl=1",
})]
// the long s is not an i, so "function" with U+017F in place of i is no keyword
#[case::long_s_in_function(Behavior::CohortFlags {
    prompt: "funct\u{17f}on",
    request_type: RequestType::General,
    expected: "general|short|code=0|math=0|mc=0|intl=1",
})]
// a fenced block sets the code flag on its own, and an "=" sets the math flag on its own
#[case::code_fence(Behavior::CohortFlags {
    prompt: "```\nlet x\n```",
    request_type: RequestType::General,
    expected: "general|short|code=1|math=0|mc=0|intl=0",
})]
#[case::math_symbol(Behavior::CohortFlags {
    prompt: "1 + 1 = 2",
    request_type: RequestType::General,
    expected: "general|short|code=0|math=1|mc=0|intl=0",
})]
// Python's \b needs a non-word character on both sides, so these letter runs stay unflagged
#[case::embedded_keyword(Behavior::CohortFlags {
    prompt: "decode the message",
    request_type: RequestType::General,
    expected: "general|short|code=0|math=0|mc=0|intl=0",
})]
#[case::underscore_keyword(Behavior::CohortFlags {
    prompt: "call_function now",
    request_type: RequestType::General,
    expected: "general|short|code=0|math=0|mc=0|intl=0",
})]
#[case::combining_mark_selects_simple(Behavior::UnicodeSelection {
    prompt: "code\u{301}",
    expected: TierNumber::One,
})]
#[case::dotless_i_selects_simple(Behavior::UnicodeSelection {
    prompt: "funct\u{131}on",
    expected: TierNumber::One,
})]
#[case::long_s_selects_complex(Behavior::UnicodeSelection {
    prompt: "funct\u{17f}on",
    expected: TierNumber::Three,
})]
#[case::short_at_199(Behavior::CohortBucket { length: 199, expected: "short" })]
#[case::medium_at_200(Behavior::CohortBucket { length: 200, expected: "medium" })]
#[case::medium_at_799(Behavior::CohortBucket { length: 799, expected: "medium" })]
#[case::long_at_800(Behavior::CohortBucket { length: 800, expected: "long" })]
#[case::long_at_1999(Behavior::CohortBucket { length: 1999, expected: "long" })]
#[case::very_long_at_2000(Behavior::CohortBucket { length: 2000, expected: "very_long" })]
#[case::empty_prompt(Behavior::EmptyPrompt)]
fn tier_artifact_predictor_behaviors(#[case] behavior: Behavior) {
    match behavior {
        Behavior::GlobalOnly => global_only(),
        Behavior::DomainPosterior => domain_posterior(),
        Behavior::CohortPosterior => cohort_posterior(),
        Behavior::Monotone => monotone(),
        Behavior::ThresholdDefault => threshold_default(),
        Behavior::ThresholdOverride => threshold_override(),
        Behavior::FirstTierAtThreshold => first_tier_at_threshold(),
        Behavior::LastTierFallback => last_tier_fallback(),
        Behavior::Signals => signals(),
        Behavior::ClassifiedTier => classified_tier(),
        Behavior::Forecast => forecast(),
        Behavior::ForecastPrecision => forecast_precision(),
        Behavior::BuiltInArtifact => builtin_artifact(),
        Behavior::Attribution => attribution(),
        Behavior::ShippedConfigSeed => shipped_config_seed(),
        Behavior::CohortFlags {
            prompt,
            request_type,
            expected,
        } => assert_eq!(cohort(prompt, request_type), expected),
        Behavior::CohortBucket { length, expected } => {
            let prompt = "a".repeat(length);
            assert_eq!(
                cohort(&prompt, RequestType::General),
                format!("general|{expected}|code=0|math=0|mc=0|intl=0")
            );
        }
        Behavior::UnicodeSelection { prompt, expected } => {
            let artifact =
                resolve_tier_artifact(&ArtifactSource::BuiltIn("ultrafeedback".to_owned()))
                    .expect("the packaged artifact validates");
            let predictor =
                TierSuccessPredictor::new(artifact, None).expect("the pinned patterns compile");
            let prediction = predictor.predict(prompt, RequestType::General);
            assert_eq!(prediction.required_tier, expected);
        }
        Behavior::EmptyPrompt => {
            assert_eq!(
                cohort("", RequestType::General),
                "general|short|code=0|math=0|mc=0|intl=0"
            );
            let outcome =
                classifier(statistics((1.0, 2.0), (1.0, 2.0), (1.0, 2.0), (1.0, 2.0))).classify("");
            assert_eq!(outcome.forecast.request_type, RequestType::General);
        }
    }
}

fn cohort(prompt: &str, request_type: RequestType) -> String {
    CohortMatcher::new()
        .expect("the pinned patterns compile")
        .similarity_cohort(prompt, request_type)
}

fn classifier(global: GlobalStatistics) -> TierArtifactPredictor {
    let predictor = TierSuccessPredictor::new(artifact_with(global), None)
        .expect("the pinned patterns compile");
    TierArtifactPredictor::new(predictor).expect("the pinned patterns compile")
}

fn global_only() {
    let prediction = predictor(statistics((0.0, 6.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0)))
        .predict("Hello.", RequestType::General);
    assert_eq!(
        prediction.probabilities.map(f64::to_bits),
        [
            0.125_f64.to_bits(),
            0.375_f64.to_bits(),
            0.5_f64.to_bits(),
            0.625_f64.to_bits(),
        ]
    );
    assert_eq!(prediction.required_tier, TierNumber::Four);
}

fn domain_posterior() {
    let fields = ", \"domain_statistics\": [{\"tier\": 2, \"successes\": 2.0, \"observations\": 6.0, \"request_type\": \"general\"}]";
    let artifact = parsed(
        &global_json([(0.0, 6.0), (0.0, 2.0), (1.0, 2.0), (4.0, 6.0)]),
        fields,
    );
    let predictor = TierSuccessPredictor::new(artifact, None).expect("the pinned patterns compile");
    let prediction = predictor.predict("Hello.", RequestType::General);
    assert_eq!(prediction.probabilities[0].to_bits(), 0.125_f64.to_bits());
    // (successes + prior_mass * global_mean) / (observations + prior_mass) is 52/206, written as its exact double
    assert_eq!(
        prediction.probabilities[1].to_bits(),
        0.2524271844660194_f64.to_bits()
    );
    assert_eq!(prediction.probabilities[2].to_bits(), 0.5_f64.to_bits());
    assert_eq!(prediction.probabilities[3].to_bits(), 0.625_f64.to_bits());
    // the domain is keyed by request type, so another type keeps the global mean
    let elsewhere = predictor.predict("Hello.", RequestType::Writing);
    assert_eq!(elsewhere.probabilities[1].to_bits(), 0.25_f64.to_bits());
}

fn cohort_posterior() {
    let fields = ", \"cohort_statistics\": [{\"tier\": 3, \"successes\": 3.0, \"observations\": 4.0, \"cohort\": \"general|short|code=0|math=0|mc=0|intl=0\"}]";
    let artifact = parsed(
        &global_json([(0.0, 6.0), (0.0, 2.0), (1.0, 2.0), (4.0, 6.0)]),
        fields,
    );
    let predictor = TierSuccessPredictor::new(artifact, None).expect("the pinned patterns compile");
    let prediction = predictor.predict("Hello.", RequestType::General);
    // (successes + cohort_prior_mass * domain_mean) / (observations + cohort_prior_mass) is 13/24
    assert_eq!(
        prediction.probabilities[2].to_bits(),
        0.5416666666666666_f64.to_bits()
    );
    // a prompt whose four flags differ builds another cohort and keeps the global mean
    let missed = predictor.predict("Describe this code.", RequestType::General);
    assert_eq!(missed.probabilities[2].to_bits(), 0.5_f64.to_bits());
}

fn monotone() {
    // the raw posterior means are 0.1875, 0.0625, 0.75 and 0.125 in tier order
    let prediction = predictor(statistics((2.0, 14.0), (0.0, 14.0), (2.0, 2.0), (0.0, 6.0)))
        .predict("Hello.", RequestType::General);
    assert_eq!(
        prediction.probabilities.map(f64::to_bits),
        [
            0.1875_f64.to_bits(),
            0.1875_f64.to_bits(),
            0.75_f64.to_bits(),
            0.75_f64.to_bits(),
        ]
    );
    assert_eq!(prediction.required_tier, TierNumber::Three);
}

fn threshold_default() {
    let predictor = predictor(statistics((0.0, 6.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0)));
    assert_eq!(predictor.threshold.to_bits(), 0.75_f64.to_bits());
}

fn threshold_override() {
    let artifact = artifact_with(statistics((0.0, 6.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0)));
    let predictor =
        TierSuccessPredictor::new(artifact, Some(0.5)).expect("the pinned patterns compile");
    assert_eq!(predictor.threshold.to_bits(), 0.5_f64.to_bits());
    assert_eq!(
        predictor
            .predict("Hello.", RequestType::General)
            .required_tier,
        TierNumber::Three
    );
}

fn first_tier_at_threshold() {
    let artifact = artifact_with(statistics((1.0, 2.0), (1.0, 2.0), (1.0, 2.0), (1.0, 2.0)));
    let predictor =
        TierSuccessPredictor::new(artifact, Some(0.5)).expect("the pinned patterns compile");
    let prediction = predictor.predict("Hello.", RequestType::General);
    assert_eq!(
        prediction.probabilities.map(f64::to_bits),
        [0.5_f64.to_bits(); 4]
    );
    assert_eq!(prediction.required_tier, TierNumber::One);
}

fn last_tier_fallback() {
    let artifact = artifact_with(statistics((0.0, 8.0), (0.0, 8.0), (0.0, 8.0), (0.0, 8.0)));
    let predictor =
        TierSuccessPredictor::new(artifact, Some(0.5)).expect("the pinned patterns compile");
    assert_eq!(
        predictor
            .predict("Hello.", RequestType::General)
            .required_tier,
        TierNumber::Four
    );
}

fn signals() {
    let outcome =
        classifier(statistics((0.0, 6.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0))).classify("Hello.");
    assert_eq!(
        outcome.signals,
        [
            "request-type:general",
            "tier-probability:simple=0.125000",
            "tier-probability:medium=0.375000",
            "tier-probability:complex=0.500000",
            "tier-probability:reasoning=0.625000",
        ]
    );
    let code = classifier(statistics((0.0, 6.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0)))
        .classify("Write a Python function that adds two numbers.");
    assert_eq!(code.signals[0], "request-type:code_generation");
}

fn classified_tier() {
    let outcome =
        classifier(statistics((0.0, 6.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0))).classify("Hello.");
    assert_eq!(outcome.tier, Tier::BuiltIn(ComplexityTier::Reasoning));
    assert_eq!(outcome.score, None);
    assert_eq!(outcome.cause, RoutingDecisionCause::HeuristicV2);
}

fn forecast() {
    let outcome =
        classifier(statistics((0.0, 6.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0))).classify("Hello.");
    assert_eq!(
        outcome.forecast.probabilities.map(f64::to_bits),
        [
            0.125_f64.to_bits(),
            0.375_f64.to_bits(),
            0.5_f64.to_bits(),
            0.625_f64.to_bits(),
        ]
    );
    assert_eq!(outcome.forecast.threshold.to_bits(), 0.75_f64.to_bits());
    assert_eq!(outcome.forecast.predicted_tier, ComplexityTier::Reasoning);
    assert_eq!(outcome.forecast.request_type, RequestType::General);
    assert_eq!(
        texts(&outcome.forecast.probabilities),
        ["0.125000", "0.375000", "0.500000", "0.625000"]
    );
}

// the forecast carries the unrounded probabilities: a six-decimal forecast shifts every one of these
fn forecast_precision() {
    let artifact = resolve_tier_artifact(&ArtifactSource::BuiltIn("ultrafeedback".to_owned()))
        .expect("the packaged artifact validates");
    let predictor = TierSuccessPredictor::new(artifact, None).expect("the pinned patterns compile");
    let outcome = TierArtifactPredictor::new(predictor)
        .expect("the pinned patterns compile")
        .classify("code\u{301}");
    assert_eq!(
        outcome.forecast.probabilities.map(f64::to_bits),
        [
            0.7926039425750072_f64.to_bits(),
            0.8701522651000183_f64.to_bits(),
            0.9020887951011245_f64.to_bits(),
            0.9844772976161567_f64.to_bits(),
        ]
    );
    assert_eq!(outcome.forecast.threshold.to_bits(), 0.75_f64.to_bits());
    assert_eq!(outcome.signals[1], "tier-probability:simple=0.792604");
}

// the shipped config seed names the packaged artifact and the model_list handoff keeps its threshold
fn shipped_config_seed() {
    let config = corpus_config(CONFIG_SEED).expect("the shipped seed loads");
    assert_eq!(
        config.artifact,
        ArtifactSource::BuiltIn("ultrafeedback".to_owned())
    );
    assert_eq!(
        config.success_threshold.map(f64::to_bits),
        Some(0.875_f64.to_bits())
    );
    let artifact =
        resolve_tier_artifact(&config.artifact).expect("the packaged artifact validates");
    let predictor = TierSuccessPredictor::new(artifact, config.success_threshold)
        .expect("the pinned patterns compile");
    let prediction = predictor.predict("Hello.", RequestType::General);
    assert_eq!(
        texts(&prediction.probabilities),
        ["0.901084", "0.934744", "0.971458", "0.991117"]
    );
    assert_eq!(prediction.required_tier, TierNumber::One);
}

fn builtin_artifact() {
    let artifact = resolve_tier_artifact(&ArtifactSource::BuiltIn("ultrafeedback".to_owned()))
        .expect("the packaged artifact validates");
    assert_eq!(
        artifact.global.one.successes.to_bits(),
        36619.0_f64.to_bits()
    );
    assert_eq!(
        artifact.global.one.observations.to_bits(),
        45504.0_f64.to_bits()
    );
    assert_eq!(
        artifact.global.four.successes.to_bits(),
        11393.0_f64.to_bits()
    );
    assert_eq!(artifact.routing_threshold.to_bits(), 0.75_f64.to_bits());
    assert_eq!(artifact.domain_prior_mass.to_bits(), 200.0_f64.to_bits());
    assert_eq!(artifact.cohort_prior_mass.to_bits(), 20.0_f64.to_bits());
    assert!(artifact.domain.contains_key(&RequestType::CodeGeneration));
    let cohort = artifact
        .cohort
        .get("analytical_reasoning|long|code=0|math=0|mc=0|intl=0")
        .expect("the packaged cohort statistics carry this cohort");
    assert_eq!(
        cohort.get(&TierNumber::Two),
        Some(&TierStatistic {
            successes: 420.0,
            observations: 519.0,
        })
    );
    assert_eq!(artifact.datasets.len(), 1);
    assert_eq!(artifact.datasets[0].rows, 255_864);
}

fn attribution() {
    let fields = ", \"datasets\": [{\"name\": \"openbmb/UltraFeedback\", \"url\": \"https://huggingface.co/datasets/openbmb/UltraFeedback\", \"license\": \"MIT\", \"rows\": 255864, \"success_definition\": \"score >= 4\"}], \"success_definition\": \"score >= 4\", \"split_method\": \"hash split\"";
    let artifact = parsed(
        &global_json([(0.0, 6.0), (2.0, 6.0), (1.0, 2.0), (4.0, 6.0)]),
        fields,
    );
    assert_eq!(artifact.datasets.len(), 1);
    assert_eq!(artifact.datasets[0].name, "openbmb/UltraFeedback");
    assert_eq!(
        artifact.datasets[0].url,
        "https://huggingface.co/datasets/openbmb/UltraFeedback"
    );
    assert_eq!(artifact.datasets[0].success_definition, "score >= 4");
    assert_eq!(artifact.success_definition, "score >= 4");
    assert_eq!(artifact.split_method, "hash split");
}
