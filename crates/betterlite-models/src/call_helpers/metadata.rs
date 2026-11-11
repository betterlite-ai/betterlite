use indexmap::IndexMap;
use serde::Deserialize;

use crate::decisions::Json;

pub type MetadataMap = IndexMap<String, Json>;

// request parsing never builds this, so a client cannot stamp a proxy-written marker
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct InternalMetadata {
    pub entries: MetadataMap,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CallParams {
    pub metadata: Option<MetadataMap>,
    pub litellm_metadata: Option<InternalMetadata>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MetadataSlots {
    pub metadata: Option<MetadataMap>,
    pub litellm_metadata: Option<MetadataMap>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataBucket {
    Metadata,
    LitellmMetadata,
}

impl MetadataBucket {
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::LitellmMetadata => "litellm_metadata",
        }
    }
}

#[must_use]
pub const fn metadata_variable_name(slots: &MetadataSlots) -> MetadataBucket {
    if slots.litellm_metadata.is_some() {
        MetadataBucket::LitellmMetadata
    } else {
        MetadataBucket::Metadata
    }
}

#[must_use]
pub fn get_or_create_metadata_bucket(
    slots: &mut MetadataSlots,
) -> (MetadataBucket, &mut MetadataMap) {
    let name = metadata_variable_name(slots);
    let bucket = match name {
        MetadataBucket::Metadata => slots.metadata.get_or_insert_with(MetadataMap::new),
        MetadataBucket::LitellmMetadata => {
            slots.litellm_metadata.get_or_insert_with(MetadataMap::new)
        }
    };
    (name, bucket)
}

pub fn add_missing_spend_metadata(litellm_metadata: &mut MetadataMap, metadata: &MetadataMap) {
    for (key, value) in metadata {
        if key.contains("user_api_key") {
            litellm_metadata.insert(key.clone(), value.clone());
        }
    }
}

#[must_use]
pub fn litellm_metadata_from_params(params: Option<&CallParams>) -> MetadataMap {
    let Some(given) = params else {
        return MetadataMap::new();
    };
    let caller_bucket = given
        .metadata
        .as_ref()
        .filter(|metadata| !metadata.is_empty());
    let internal_bucket = given
        .litellm_metadata
        .as_ref()
        .map(|internal| &internal.entries)
        .filter(|entries| !entries.is_empty());
    match (internal_bucket, caller_bucket) {
        (Some(internal), Some(caller)) => {
            let mut merged = internal.clone();
            add_missing_spend_metadata(&mut merged, caller);
            merged
        }
        (Some(internal), None) => internal.clone(),
        (None, Some(caller)) => caller.clone(),
        (None, None) => MetadataMap::new(),
    }
}

#[must_use]
pub fn stamped_oauth_token<'request>(
    metadata: Option<&'request MetadataMap>,
    params: Option<&'request CallParams>,
) -> Option<&'request Json> {
    if let Some(value) = params
        .and_then(|given| given.litellm_metadata.as_ref())
        .and_then(|internal| internal.entries.get("used_client_oauth_token"))
    {
        return Some(value);
    }
    let caller = metadata?;
    caller.get("used_client_oauth_token")
}
