use serde::{Deserialize, Serialize};

use super::error::ForecastError;
use crate::capability_math;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseFormat {
    #[default]
    JsonSchema,
    JsonObject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    pub version: String,
    pub slope: f64,
    pub intercept: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub efficient_tier: String,
    pub capable_tier: String,
    pub base_threshold: f64,
    pub threshold_step: f64,
    pub max_output_tokens: u32,
    pub calibration: Option<Calibration>,
    pub response_format: ResponseFormat,
}

impl Config {
    pub fn validate(mut self) -> Result<Self, ForecastError> {
        self.efficient_tier = self.efficient_tier.trim().to_owned();
        if self.efficient_tier.is_empty() {
            return Err(ForecastError::EmptyEfficientTier);
        }
        self.capable_tier = self.capable_tier.trim().to_owned();
        if self.capable_tier.is_empty() {
            return Err(ForecastError::EmptyCapableTier);
        }
        if !(self.base_threshold.is_finite() && (0.0_f64..=1.0_f64).contains(&self.base_threshold))
        {
            return Err(ForecastError::BaseThreshold {
                value: self.base_threshold,
            });
        }
        if !(self.threshold_step.is_finite() && self.threshold_step >= 0.0_f64) {
            return Err(ForecastError::ThresholdStep {
                value: self.threshold_step,
            });
        }
        if capability_math::max_threshold(self.base_threshold, self.threshold_step).is_none() {
            return Err(ForecastError::ThresholdRange);
        }
        if self.max_output_tokens == 0 {
            return Err(ForecastError::MaxOutputTokens);
        }
        self.calibration = self.calibration.map(Calibration::validate).transpose()?;
        Ok(self)
    }
}

impl Calibration {
    fn validate(self) -> Result<Self, ForecastError> {
        let characters = self.version.chars().count();
        if characters == 0
            || characters > 128
            || self.version.trim() != self.version
            || self.version.contains('\n')
        {
            return Err(ForecastError::CalibrationVersion);
        }
        if !(self.slope.is_finite() && (0.0_f64..=20.0_f64).contains(&self.slope)) {
            return Err(ForecastError::CalibrationSlope { value: self.slope });
        }
        if !(self.intercept.is_finite() && (-20.0_f64..=20.0_f64).contains(&self.intercept)) {
            return Err(ForecastError::CalibrationIntercept {
                value: self.intercept,
            });
        }
        Ok(self)
    }
}
