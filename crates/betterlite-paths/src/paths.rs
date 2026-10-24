pub mod delete;
pub mod lookup;
pub mod segments;

use serde::{Deserialize, Serialize};

use crate::error::PathError;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Document {
    #[expect(
        clippy::disallowed_types,
        reason = "JWT claims and provider parameters are operator-addressed JSON that betterlite does not model"
    )]
    pub json: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathLimits {
    pub max_path_bytes: usize,
    pub max_depth: usize,
    pub max_selectors: usize,
    pub max_visits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Field(String),
    Wildcard,
    Index(usize),
    // a bracket whose content Python's int() refuses, or a negative index; it matches nothing
    Unmatched,
}

impl PathLimits {
    const fn check_length(&self, path: &str) -> Result<(), PathError> {
        if path.len() > self.max_path_bytes {
            return Err(PathError::TooLong {
                bytes: path.len(),
                limit: self.max_path_bytes,
            });
        }
        Ok(())
    }

    const fn check_depth(&self, depth: usize) -> Result<(), PathError> {
        if depth > self.max_depth {
            return Err(PathError::TooDeep {
                limit: self.max_depth,
            });
        }
        Ok(())
    }
}

// a derived Eq would name the JSON type again outside the field's expect
impl Eq for Document {}
