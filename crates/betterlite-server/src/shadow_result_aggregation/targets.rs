use std::cmp::Ordering;

use super::error::ShadowResultsError;
use crate::shadow_eval_models::ShadowEvalTargetType;

// one target identity; a key, team and user may share an id, so the kind is part of the key
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetKey {
    pub target_type: ShadowEvalTargetType,
    pub target_id: String,
}

impl Ord for TargetKey {
    fn cmp(&self, other: &Self) -> Ordering {
        target_type_name(self.target_type)
            .cmp(target_type_name(other.target_type))
            .then_with(|| self.target_id.cmp(&other.target_id))
    }
}

impl PartialOrd for TargetKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// the Literal spelling LiteLLM sorts and serializes targets by
#[must_use]
pub const fn target_type_name(target_type: ShadowEvalTargetType) -> &'static str {
    match target_type {
        ShadowEvalTargetType::Key => "key",
        ShadowEvalTargetType::Team => "team",
        ShadowEvalTargetType::User => "user",
    }
}

pub fn parse_target_type(raw: &str) -> Result<ShadowEvalTargetType, ShadowResultsError> {
    match raw {
        "key" => Ok(ShadowEvalTargetType::Key),
        "team" => Ok(ShadowEvalTargetType::Team),
        "user" => Ok(ShadowEvalTargetType::User),
        other => Err(ShadowResultsError::UnknownTargetType {
            target_type: other.to_owned(),
        }),
    }
}
