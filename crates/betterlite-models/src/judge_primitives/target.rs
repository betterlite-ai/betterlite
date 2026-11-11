use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JudgeVia {
    Router,
    Sdk,
    Nothing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JudgeTarget {
    pub via: JudgeVia,
    pub models: BTreeSet<String>,
}
