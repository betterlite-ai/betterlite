use std::collections::BTreeMap;
use std::fmt::{self, Formatter};

use betterlite_config::sections::tier_artifact_predictor::ArtifactSource;
use betterlite_config::sections::tier_artifact_predictor::artifact::{Artifact, Dataset};

use crate::adaptive_classification::request_type::RequestType;
use crate::complexity_scoring::tier::ComplexityTier;
use crate::tier_artifact_predictor::error::PredictorError;
use crate::tier_artifact_predictor::validation;

// bounded-inputs: a packaged or configured artifact is decoded only inside this many bytes
pub const MAX_ARTIFACT_BYTES: usize = 1_048_576;

// the pinned artifact ships with the crate, so a changing download never replaces it
const BUILT_IN_ARTIFACTS: [(&str, &[u8]); 1] = [(
    "ultrafeedback",
    include_bytes!("fixtures/ultrafeedback_tiers.json"),
)];

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

impl fmt::Display for TierNumber {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.built_in().name())
    }
}

// the reference keys global statistics by tier, so the validated form names the four slots
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

// resolve_tier_artifact: a configured name reads the packaged artifact; an inline artifact is used as given
pub fn resolve_tier_artifact(source: &ArtifactSource) -> Result<ValidatedArtifact, PredictorError> {
    match source {
        ArtifactSource::BuiltIn(name) => match BUILT_IN_ARTIFACTS
            .iter()
            .find(|(artifact_name, _bytes)| *artifact_name == name)
        {
            Some((_artifact_name, bytes)) => parse_artifact(bytes),
            None => Err(PredictorError::UnknownArtifact { name: name.clone() }),
        },
        ArtifactSource::Inline(artifact) => validation::validate(artifact),
    }
}

// TrainedTierArtifact.model_validate_json behind the bounded-inputs limit
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
