use crate::adaptive_classification::classifier::Classifier;
use crate::adaptive_classification::request_type::RequestType;
use crate::complexity_scoring::decision::RoutingDecisionCause;
use crate::complexity_scoring::tier::{ComplexityTier, Tier};
use crate::posterior_math;
use crate::tier_artifact_predictor::artifact::{TierNumber, ValidatedArtifact};
use crate::tier_artifact_predictor::cohort::CohortMatcher;
use crate::tier_artifact_predictor::error::PredictorError;

#[derive(Debug, Clone, PartialEq)]
pub struct TierPrediction {
    pub probabilities: [f64; 4],
    pub required_tier: TierNumber,
}

// StandardLoggingHeuristicV2Forecast: the four tier probabilities, the applied threshold and the choice
#[derive(Debug, Clone, PartialEq)]
pub struct TierForecast {
    pub probabilities: [f64; 4],
    pub threshold: f64,
    pub predicted_tier: ComplexityTier,
    pub request_type: RequestType,
}

// ClassificationOutcome for classifier_type heuristic_v2: no score, five signals and the forecast
#[derive(Debug, Clone, PartialEq)]
pub struct ClassifierOutcome {
    pub tier: Tier,
    pub score: Option<f64>,
    pub signals: Vec<String>,
    pub cause: RoutingDecisionCause,
    pub forecast: TierForecast,
}

pub struct TierSuccessPredictor {
    pub threshold: f64,
    artifact: ValidatedArtifact,
    matcher: CohortMatcher,
}

impl TierSuccessPredictor {
    // __init__: an absent override keeps the artifact's routing_threshold
    pub fn new(
        artifact: ValidatedArtifact,
        threshold: Option<f64>,
    ) -> Result<Self, PredictorError> {
        let applied = threshold.unwrap_or(artifact.routing_threshold);
        Ok(Self {
            threshold: applied,
            artifact,
            matcher: CohortMatcher::new()?,
        })
    }

    // predict: the running maximum over the four raw posterior means, then the first tier at the threshold
    #[must_use]
    pub fn predict(&self, prompt: &str, request_type: RequestType) -> TierPrediction {
        let cohort = self.matcher.similarity_cohort(prompt, request_type);
        let raw = TierNumber::ALL.map(|tier| self.probability(tier, request_type, &cohort));
        let mut highest = f64::NEG_INFINITY;
        let probabilities = raw.map(|probability| {
            highest = highest.max(probability);
            highest
        });
        let required_tier = TierNumber::ALL
            .iter()
            .zip(probabilities.iter())
            .find(|(_tier, probability)| **probability >= self.threshold)
            .map_or(TierNumber::Four, |(tier, _probability)| *tier);
        TierPrediction {
            probabilities,
            required_tier,
        }
    }

    // _probability: the global mean, then the domain posterior, then the cohort posterior
    fn probability(&self, tier: TierNumber, request_type: RequestType, cohort: &str) -> f64 {
        let global = self.artifact.global.for_tier(tier);
        let global_mean = posterior_math::smoothed_mean(global.successes, global.observations);
        let domain = self
            .artifact
            .domain
            .get(&request_type)
            .and_then(|tiers| tiers.get(&tier));
        let domain_mean = domain.map_or(global_mean, |statistic| {
            posterior_math::posterior_mean(
                statistic.successes,
                statistic.observations,
                self.artifact.domain_prior_mass,
                global_mean,
            )
        });
        let cohort_statistic = self
            .artifact
            .cohort
            .get(cohort)
            .and_then(|tiers| tiers.get(&tier));
        cohort_statistic.map_or(domain_mean, |statistic| {
            posterior_math::posterior_mean(
                statistic.successes,
                statistic.observations,
                self.artifact.cohort_prior_mass,
                domain_mean,
            )
        })
    }
}

pub struct TierArtifactPredictor {
    predictor: TierSuccessPredictor,
    classifier: Classifier,
}

impl TierArtifactPredictor {
    pub fn new(
        artifact: ValidatedArtifact,
        threshold: Option<f64>,
    ) -> Result<Self, PredictorError> {
        Ok(Self {
            predictor: TierSuccessPredictor::new(artifact, threshold)?,
            classifier: Classifier::new()
                .map_err(|source| PredictorError::Classifier { source })?,
        })
    }

    // _classify_with_heuristic_v2: the pinned request-type classifier feeds the artifact predictor
    #[must_use]
    pub fn classify(&self, prompt: &str) -> ClassifierOutcome {
        let request_type = self.classifier.classify_prompt(prompt);
        let prediction = self.predictor.predict(prompt, request_type);
        let predicted_tier = prediction.required_tier.built_in();
        let mut signals = Vec::with_capacity(TierNumber::ALL.len().saturating_add(1));
        signals.push(format!("request-type:{request_type}"));
        for (tier, probability) in TierNumber::ALL.iter().zip(prediction.probabilities.iter()) {
            let name = tier.built_in().name().to_ascii_lowercase();
            signals.push(format!("tier-probability:{name}={probability:.6}"));
        }
        ClassifierOutcome {
            tier: Tier::BuiltIn(predicted_tier),
            score: None,
            signals,
            cause: RoutingDecisionCause::HeuristicV2,
            forecast: TierForecast {
                probabilities: prediction.probabilities,
                threshold: self.predictor.threshold,
                predicted_tier,
                request_type,
            },
        }
    }
}
