use std::collections::BTreeMap;

use crate::field::Field;
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnNull, serde_as};

#[expect(
    clippy::disallowed_types,
    reason = "Unmodeled AWS request and tag fields must survive owner-tag injection"
)]
pub type PassthroughFields = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct StartTag {
    #[serde(default, skip_serializing_if = "Field::is_absent")]
    pub key: Field<String>,
    #[serde(default, skip_serializing_if = "Field::is_absent")]
    pub value: Field<String>,
    #[serde(flatten)]
    pub passthrough: PassthroughFields,
}

#[serde_as]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct OwnedStartRequest {
    #[serde(default)]
    #[serde_as(as = "DefaultOnNull")]
    pub tags: Vec<StartTag>,
    #[serde(flatten)]
    pub passthrough: PassthroughFields,
}
