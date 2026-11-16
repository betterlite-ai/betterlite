use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use super::{Ignore, validate_ignores};
use crate::error::HelperRunError;
use crate::helpers::HelperCall;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperCase {
    pub kind: HelperKind,
    pub name: String,
    pub call: HelperCall,
    // the same case checks these after `call`, so one case file covers several inputs
    #[serde(default)]
    pub calls: Vec<HelperCall>,
    pub env: BTreeMap<String, String>,
    pub compare: HelperCompare,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperKind {
    Helper,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperCompare {
    pub ignore: Vec<Ignore>,
}

pub fn validate_helper(path: &Path, case: &HelperCase) -> Result<(), HelperRunError> {
    let invalid = |message: &str| HelperRunError::CaseInvalid {
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

pub fn parse_helper_case(path: &Path, text: &str) -> Result<HelperCase, HelperRunError> {
    let case: HelperCase =
        serde_saphyr::from_str(text).map_err(|source| HelperRunError::CaseParse {
            path: path.to_owned(),
            source: Box::new(source),
        })?;
    validate_helper(path, &case)?;
    Ok(case)
}
