use std::collections::BTreeMap;

use super::error::AzureContentSafetyError;
use super::legacy_wire::TextCategory;

pub const DEFAULT_THRESHOLD: i64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeverityThresholds {
    pub hate: i64,
    pub self_harm: i64,
    pub sexual: i64,
    pub violence: i64,
}

impl Default for SeverityThresholds {
    fn default() -> Self {
        Self {
            hate: DEFAULT_THRESHOLD,
            self_harm: DEFAULT_THRESHOLD,
            sexual: DEFAULT_THRESHOLD,
            violence: DEFAULT_THRESHOLD,
        }
    }
}

impl SeverityThresholds {
    #[must_use]
    pub const fn for_category(&self, category: TextCategory) -> i64 {
        match category {
            TextCategory::Hate => self.hate,
            TextCategory::SelfHarm => self.self_harm,
            TextCategory::Sexual => self.sexual,
            TextCategory::Violence => self.violence,
        }
    }

    const fn set(&mut self, category: TextCategory, threshold: i64) {
        match category {
            TextCategory::Hate => self.hate = threshold,
            TextCategory::SelfHarm => self.self_harm = threshold,
            TextCategory::Sexual => self.sexual = threshold,
            TextCategory::Violence => self.violence = threshold,
        }
    }
}

pub fn configure_thresholds(
    thresholds: BTreeMap<String, i64>,
) -> Result<SeverityThresholds, AzureContentSafetyError> {
    let mut configured = SeverityThresholds::default();
    for (name, threshold) in thresholds {
        let Some(category) = TextCategory::from_name(&name) else {
            return Err(AzureContentSafetyError::UnknownCategory { category: name });
        };
        configured.set(category, threshold);
    }
    Ok(configured)
}
