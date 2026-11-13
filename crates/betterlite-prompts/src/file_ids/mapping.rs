use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct FileIdMapping {
    pub(crate) entries: BTreeMap<String, BTreeMap<String, String>>,
}

impl FileIdMapping {
    #[must_use]
    pub(crate) fn provider_file_id(&self, file_id: &str, model_id: &str) -> Option<&str> {
        self.entries
            .get(file_id)
            .and_then(|models| models.get(model_id))
            .map(String::as_str)
            .filter(|provider| !provider.is_empty())
    }
}
