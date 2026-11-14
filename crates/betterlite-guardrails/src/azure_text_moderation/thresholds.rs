use crate::azure_content_safety::legacy_wire::TextCategory;

use super::AzureTextModerationGuardrail;
use super::config::DEFAULT_SEVERITY_THRESHOLD;
use super::error::AzureTextModerationError;
use super::wire::{AnalyzeResponse, CategoryAnalysis};

impl AzureTextModerationGuardrail {
    // LiteLLM checks a category map first, then a global threshold, then the default.
    pub fn check_severity_threshold(
        &self,
        response: &AnalyzeResponse,
    ) -> Result<(), AzureTextModerationError> {
        if let Some(thresholds) = self.severity_threshold_by_category.as_ref() {
            for analysis in &response.categories_analysis {
                let category_threshold = TextCategory::from_name(&analysis.category)
                    .and_then(|category| thresholds.get(&category).copied());
                if let Some(category_threshold) = category_threshold {
                    violation_at(analysis, category_threshold)?;
                }
            }
        }
        if let Some(threshold) = self.severity_threshold {
            for analysis in &response.categories_analysis {
                violation_at(analysis, threshold)?;
            }
        }
        if self.severity_threshold.is_none() && self.severity_threshold_by_category.is_none() {
            for analysis in &response.categories_analysis {
                violation_at(analysis, DEFAULT_SEVERITY_THRESHOLD)?;
            }
        }
        Ok(())
    }
}

fn violation_at(
    analysis: &CategoryAnalysis,
    threshold: i64,
) -> Result<(), AzureTextModerationError> {
    if analysis.severity < threshold {
        return Ok(());
    }
    Err(AzureTextModerationError::Violation {
        category: analysis.category.clone(),
        threshold,
        severity: analysis.severity,
    })
}
