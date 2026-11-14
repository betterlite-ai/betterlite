use crate::budget_permission_rows::JsonData;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbRecord {
    #[serde(default)]
    pub model_dump: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub dict: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub mapping: Option<BTreeMap<String, JsonData>>,
    #[serde(default)]
    pub pairs: Option<Vec<(String, JsonData)>>,
}

impl DbRecord {
    #[must_use]
    pub fn project(&self) -> BTreeMap<String, JsonData> {
        if let Some(fields) = &self.model_dump {
            return fields.clone();
        }
        if let Some(fields) = &self.dict {
            return fields.clone();
        }
        if let Some(fields) = &self.mapping {
            return fields.clone();
        }
        self.pairs
            .as_ref()
            .map_or_else(BTreeMap::new, |pairs| pairs.iter().cloned().collect())
    }
}
