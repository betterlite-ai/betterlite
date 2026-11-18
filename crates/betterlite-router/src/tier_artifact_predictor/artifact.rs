use std::collections::BTreeMap;

use betterlite_config::sections::tier_artifact_predictor::ArtifactSource;
use betterlite_config::sections::tier_artifact_predictor::artifact::{Artifact, Dataset};

use crate::adaptive_classification::request_type::RequestType;
use crate::complexity_scoring::tier::ComplexityTier;
use crate::tier_artifact_predictor::error::PredictorError;
use crate::tier_artifact_predictor::validation;

pub const MAX_ARTIFACT_BYTES: usize = 0x0010_0000;

pub type TierStatistics = BTreeMap<TierNumber, TierStatistic>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TierStatistic {
    pub successes: f64,
    pub observations: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TierNumber {
    One,
    Two,
    Three,
    Four,
}

impl TierNumber {
    pub const ALL: [Self; 4] = [Self::One, Self::Two, Self::Three, Self::Four];

    #[must_use]
    pub const fn built_in(self) -> ComplexityTier {
        match self {
            Self::One => ComplexityTier::Simple,
            Self::Two => ComplexityTier::Medium,
            Self::Three => ComplexityTier::Complex,
            Self::Four => ComplexityTier::Reasoning,
        }
    }

    pub(crate) const fn declared(tier: u32) -> Result<Self, PredictorError> {
        match tier {
            1 => Ok(Self::One),
            2 => Ok(Self::Two),
            3 => Ok(Self::Three),
            4 => Ok(Self::Four),
            _ => Err(PredictorError::TierRange { tier }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalStatistics {
    pub one: TierStatistic,
    pub two: TierStatistic,
    pub three: TierStatistic,
    pub four: TierStatistic,
}

impl GlobalStatistics {
    #[must_use]
    pub const fn for_tier(&self, tier: TierNumber) -> TierStatistic {
        match tier {
            TierNumber::One => self.one,
            TierNumber::Two => self.two,
            TierNumber::Three => self.three,
            TierNumber::Four => self.four,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedArtifact {
    pub global: GlobalStatistics,
    pub domain: BTreeMap<RequestType, TierStatistics>,
    pub cohort: BTreeMap<String, TierStatistics>,
    pub domain_prior_mass: f64,
    pub cohort_prior_mass: f64,
    pub routing_threshold: f64,
    pub datasets: Vec<Dataset>,
    pub success_definition: String,
    pub split_method: String,
}

pub fn resolve_tier_artifact(source: &ArtifactSource) -> Result<ValidatedArtifact, PredictorError> {
    match source {
        ArtifactSource::BuiltIn(name) => match name.as_str() {
            "ultrafeedback" => parse_artifact(include_bytes!("fixtures/ultrafeedback_tiers.json")),
            _ => Err(PredictorError::UnknownArtifact { name: name.clone() }),
        },
        ArtifactSource::Inline(artifact) => validation::validate(artifact),
    }
}

pub fn parse_artifact(bytes: &[u8]) -> Result<ValidatedArtifact, PredictorError> {
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(PredictorError::ArtifactTooLarge {
            bytes: bytes.len(),
            limit: MAX_ARTIFACT_BYTES,
        });
    }
    let artifact = serde_json::from_slice::<Artifact>(bytes)
        .map_err(|source| PredictorError::ArtifactJson { source })?;
    validation::validate(&artifact)
}
