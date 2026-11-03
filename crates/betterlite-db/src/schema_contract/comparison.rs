use std::collections::BTreeMap;

use super::Fact;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Missing {
        expected: Option<String>,
    },
    Extra {
        actual: Option<String>,
    },
    Changed {
        expected: Option<String>,
        actual: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub differences: BTreeMap<Fact, Change>,
    pub optional_expected: BTreeMap<Fact, Option<String>>,
    pub optional_actual: BTreeMap<Fact, Option<String>>,
}

pub(super) fn difference(
    expected: Option<&Option<String>>,
    actual: Option<&Option<String>>,
) -> Option<Change> {
    match (expected, actual) {
        (Some(left), Some(right)) if left != right => Some(Change::Changed {
            expected: left.clone(),
            actual: right.clone(),
        }),
        (Some(value), None) => Some(Change::Missing {
            expected: value.clone(),
        }),
        (None, Some(value)) => Some(Change::Extra {
            actual: value.clone(),
        }),
        (Some(_), Some(_)) | (None, None) => None,
    }
}
