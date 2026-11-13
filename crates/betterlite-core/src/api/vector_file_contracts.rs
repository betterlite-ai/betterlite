use betterlite_openai::api::vector_file_contracts::records::FileParams;

mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileParamsKind {
    Create,
    List,
    Update,
}

impl FileParamsKind {
    const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Create => &["file_id", "attributes", "chunking_strategy"],
            Self::List => &["after", "before", "filter", "limit", "order"],
            Self::Update => &["attributes"],
        }
    }
}

#[must_use]
pub fn filter_file_params(kind: FileParamsKind, params: &FileParams) -> FileParams {
    params
        .iter()
        .filter(|(key, value)| kind.keys().contains(&key.as_str()) && !value.is_null())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
