use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use super::{Ignore, validate_ignores};
use crate::OracleError;
use crate::helpers::HelperCall;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperCase {
    pub(crate) kind: HelperKind,
    pub(crate) name: String,
    pub(crate) call: HelperCall,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) compare: HelperCompare,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HelperKind {
    Helper,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HelperCompare {
    pub(crate) ignore: Vec<Ignore>,
}

pub(crate) fn validate_helper(path: &Path, case: &HelperCase) -> Result<(), OracleError> {
    let invalid = |message: &str| OracleError::CaseInvalid {
        path: path.to_owned(),
        message: message.to_owned(),
    };
    if !matches!(&case.kind, HelperKind::Helper) {
        return Err(invalid("helper cases must have kind: helper"));
    }
    if case.name.is_empty() {
        return Err(invalid("name cannot be empty"));
    }
    validate_ignores(path, &case.compare.ignore)
}
