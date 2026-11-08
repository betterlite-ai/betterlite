use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::decisions::Json;

pub const RESERVED_TOP_LEVEL_KEYS: [&str; 2] = ["sample_spec", "fallback_generalizations"];
pub const MIN_MODEL_COUNT: usize = 50;
pub const MAX_SHRINK_RATIO: Decimal = Decimal::from_parts(5, 0, 0, false, 1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegrityPolicy {
    pub min_model_count: usize,
    pub max_shrink_ratio: Decimal,
}

impl Default for IntegrityPolicy {
    fn default() -> Self {
        Self {
            min_model_count: MIN_MODEL_COUNT,
            max_shrink_ratio: MAX_SHRINK_RATIO,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IntegrityError {
    #[error("the fetched model cost map is empty")]
    Empty,
    #[error("the fetched model cost map has {count} entries, below the minimum of {minimum}")]
    TooFewModels { count: usize, minimum: usize },
    #[error("the fetched model cost map has {count} entries, down from a backup of {backup}")]
    Shrunk { count: usize, backup: usize },
}

#[must_use]
pub fn count_model_entries(models: &IndexMap<String, Json>) -> usize {
    models
        .keys()
        .filter(|name| !RESERVED_TOP_LEVEL_KEYS.contains(&name.as_str()))
        .count()
}

pub fn check_model_count_not_reduced(
    models: &IndexMap<String, Json>,
    backup_model_count: usize,
    policy: IntegrityPolicy,
) -> Result<(), IntegrityError> {
    let count = count_model_entries(models);
    if count < policy.min_model_count {
        return Err(IntegrityError::TooFewModels {
            count,
            minimum: policy.min_model_count,
        });
    }
    let threshold = Decimal::from(backup_model_count).checked_mul(policy.max_shrink_ratio);
    let shrunk = threshold.is_none_or(|limit| Decimal::from(count) < limit);
    if backup_model_count != 0 && shrunk {
        return Err(IntegrityError::Shrunk {
            count,
            backup: backup_model_count,
        });
    }
    Ok(())
}

pub fn validate_model_cost_map(
    models: &IndexMap<String, Json>,
    backup_model_count: usize,
    policy: IntegrityPolicy,
) -> Result<(), IntegrityError> {
    if models.is_empty() {
        return Err(IntegrityError::Empty);
    }
    check_model_count_not_reduced(models, backup_model_count, policy)
}
