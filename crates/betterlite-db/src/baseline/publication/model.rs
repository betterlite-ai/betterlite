use betterlite_models::savings_evidence::spend::BaselineCosts;
use betterlite_models::savings_pricing::compare::ComparisonProvenance;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub(super) const VERSION: i64 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationStatus {
    Estimated,
    Unknown,
}

// The provenance literals the stored publication carries; the pricing enum itself does not serialize.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationProvenance {
    ObservedIdentical,
    Modeled,
}

impl From<ComparisonProvenance> for PublicationProvenance {
    fn from(value: ComparisonProvenance) -> Self {
        match value {
            ComparisonProvenance::ObservedIdentical => Self::ObservedIdentical,
            ComparisonProvenance::Modeled => Self::Modeled,
        }
    }
}

// BaselinePublication: a version-3 estimate of one comparison, or the unknown record that replaces it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BaselinePublication {
    pub version: i64,
    pub comparison_id: String,
    pub comparison_started_at: f64,
    pub status: PublicationStatus,
    pub reason: String,
    pub provenance: Option<PublicationProvenance>,
    #[serde(with = "rust_decimal::serde::float_option")]
    pub actual_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::float_option")]
    pub baseline_spend: Option<Decimal>,
    pub input_tokens: Option<i64>,
    pub cache_read_input_tokens: Option<i64>,
    pub cache_creation_5m_input_tokens: Option<i64>,
    pub cache_creation_1h_input_tokens: Option<i64>,
}

impl BaselinePublication {
    // costs: the two amounts both present, and only while the publication is still an estimate.
    #[must_use]
    pub fn costs(&self) -> Option<BaselineCosts> {
        if self.status != PublicationStatus::Estimated {
            return None;
        }
        Some(BaselineCosts {
            actual: self.actual_spend?,
            baseline: self.baseline_spend?,
        })
    }
}
