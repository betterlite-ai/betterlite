use betterlite_models::savings_evidence::spend::BaselineCosts;
use betterlite_models::savings_pricing::compare::ComparisonProvenance;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;
use serde::ser::SerializeStruct as _;

const VERSION: i64 = 3;
const FIELDS: usize = 11;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicationStatus {
    Estimated,
    Unknown,
}

impl PublicationStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Estimated => "estimated",
            Self::Unknown => "unknown",
        }
    }
}

// BaselinePublication: a version-3 estimate of one comparison, or the unknown record that replaces it.
#[derive(Clone, Debug, PartialEq)]
pub struct BaselinePublication {
    pub comparison_id: String,
    pub comparison_started_at: f64,
    pub status: PublicationStatus,
    pub reason: String,
    pub provenance: Option<ComparisonProvenance>,
    pub actual_spend: Option<Decimal>,
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

impl Serialize for BaselinePublication {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("BaselinePublication", FIELDS)?;
        state.serialize_field("version", &VERSION)?;
        state.serialize_field("comparison_id", &self.comparison_id)?;
        state.serialize_field("comparison_started_at", &self.comparison_started_at)?;
        state.serialize_field("status", self.status.as_str())?;
        state.serialize_field("reason", &self.reason)?;
        state.serialize_field("provenance", &self.provenance.map(provenance_text))?;
        state.serialize_field("actual_spend", &self.actual_spend.map(FloatAmount))?;
        state.serialize_field("baseline_spend", &self.baseline_spend.map(FloatAmount))?;
        state.serialize_field("input_tokens", &self.input_tokens)?;
        state.serialize_field("cache_read_input_tokens", &self.cache_read_input_tokens)?;
        state.serialize_field(
            "cache_creation_5m_input_tokens",
            &self.cache_creation_5m_input_tokens,
        )?;
        state.serialize_field(
            "cache_creation_1h_input_tokens",
            &self.cache_creation_1h_input_tokens,
        )?;
        state.end()
    }
}

const fn provenance_text(value: ComparisonProvenance) -> &'static str {
    match value {
        ComparisonProvenance::ObservedIdentical => "observed_identical",
        ComparisonProvenance::Modeled => "modeled",
    }
}

// The source prints these as Python floats; the stored value stays Decimal until this one conversion.
struct FloatAmount(Decimal);

impl Serialize for FloatAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_f64(self.0.to_f64().unwrap_or_default())
    }
}
