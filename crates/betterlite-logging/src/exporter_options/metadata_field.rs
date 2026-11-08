use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

pub const LITELLM_METADATA_FIELD: &str = "litellm_metadata";
pub const OLD_LITELLM_METADATA_FIELD: &str = "metadata";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataField {
    LitellmMetadata,
    Metadata,
}

impl MetadataField {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LitellmMetadata => LITELLM_METADATA_FIELD,
            Self::Metadata => OLD_LITELLM_METADATA_FIELD,
        }
    }
}

#[must_use]
pub fn select_metadata_field(
    request_kwargs: Option<&IndexMap<String, Json>>,
) -> Option<MetadataField> {
    let kwargs = request_kwargs?;
    if kwargs.contains_key(LITELLM_METADATA_FIELD) {
        return Some(MetadataField::LitellmMetadata);
    }
    Some(MetadataField::Metadata)
}
