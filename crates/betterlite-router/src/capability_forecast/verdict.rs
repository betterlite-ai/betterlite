use serde::{Deserialize, Serialize};

use super::error::ForecastError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rule {
    #[serde(rename = "SUP-1")]
    Sup1,
    #[serde(rename = "SUP-2")]
    Sup2,
    #[serde(rename = "SUP-3")]
    Sup3,
    #[serde(rename = "SUP-4")]
    Sup4,
    #[serde(rename = "SUP-5")]
    Sup5,
    #[serde(rename = "UNC-1")]
    Unc1,
    #[serde(rename = "UNC-2")]
    Unc2,
    #[serde(rename = "LIM-1")]
    Lim1,
    #[serde(rename = "LIM-2")]
    Lim2,
    #[serde(rename = "none")]
    None,
}

impl Rule {
    #[must_use]
    pub const fn boundary(self) -> Boundary {
        match self {
            Self::Sup1 | Self::Sup2 | Self::Sup3 | Self::Sup4 | Self::Sup5 => Boundary::Supported,
            Self::Unc1 | Self::Unc2 => Boundary::Uncertain,
            Self::Lim1 | Self::Lim2 => Boundary::Unsupported,
            Self::None => Boundary::Unmatched,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Boundary {
    Supported,
    Uncertain,
    Unsupported,
    Unmatched,
}

impl Boundary {
    #[must_use]
    pub const fn extra_steps(self) -> u8 {
        match self {
            Self::Supported => 0,
            Self::Uncertain | Self::Unmatched => 1,
            Self::Unsupported => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    pub crux: String,
    pub primary_rule: Rule,
    pub capability_boundary: Boundary,
    pub p_solve: f64,
}

impl Verdict {
    pub fn validate(self) -> Result<Self, ForecastError> {
        if self.crux.trim().is_empty() {
            return Err(ForecastError::BlankCrux);
        }
        let expected = self.primary_rule.boundary();
        if self.capability_boundary != expected {
            return Err(ForecastError::RuleBoundary {
                rule: self.primary_rule,
                expected,
                found: self.capability_boundary,
            });
        }
        if !self.p_solve.is_finite() || self.p_solve < 0.0_f64 || self.p_solve > 1.0_f64 {
            return Err(ForecastError::SolveProbability {
                value: self.p_solve,
            });
        }
        Ok(self)
    }
}
