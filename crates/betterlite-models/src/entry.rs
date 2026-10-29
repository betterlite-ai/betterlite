use crate::capabilities::CapabilityFields;
use crate::capabilities::media::MediaCapabilities;
use crate::capabilities::reasoning::ReasoningCapabilities;
use crate::capabilities::tooling::ToolingCapabilities;
use crate::capabilities::vendor::VendorCapabilities;
use crate::metadata::MetadataFields;
use crate::metadata::identity::Mode;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub mod generalizations;
mod unknown;

// B204's price groups compose beside this into the record B193 deserializes; not pulled in here.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelFields {
    #[serde(flatten)]
    pub metadata: MetadataFields,
    #[serde(flatten)]
    pub capabilities: CapabilityFields,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelFieldError {
    #[error("model field {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("model field {0}: unknown field")]
    Unknown(String),
}

// litellm_provider and mode stay plain validated data; mapping to an executable provider is its own crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderProjection<'fields> {
    pub litellm_provider: &'fields str,
    pub mode: Option<Mode>,
}

impl ModelFields {
    // Accepts an unrecognized field and reports its path so a remote refresh stays compatible.
    pub fn parse(input: &RawValue) -> Result<(Self, Vec<String>), ModelFieldError> {
        let fields = Self::decode(input)?;
        let warnings = unknown::unknown_paths(input, true);
        Ok((fields, warnings))
    }

    // Only the bundled-map inventory tests use this; runtime parsing always accepts and warns.
    pub fn parse_strict(input: &RawValue) -> Result<Self, ModelFieldError> {
        let fields = Self::decode(input)?;
        if let Some(path) = unknown::unknown_paths(input, true).into_iter().next() {
            return Err(ModelFieldError::Unknown(path));
        }
        Ok(fields)
    }

    #[must_use]
    pub fn provider_projection(&self) -> ProviderProjection<'_> {
        let litellm_provider = self.metadata.litellm_provider.as_str();
        ProviderProjection {
            litellm_provider,
            mode: self.metadata.mode,
        }
    }

    fn decode(input: &RawValue) -> Result<Self, ModelFieldError> {
        let mut deserializer = serde_json::Deserializer::from_str(input.get());
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
            let path = error.path().to_string();
            if path == "."
                && let Some(located) = Self::locate_flat_field_error(input)
            {
                return located;
            }
            ModelFieldError::Json {
                path,
                source: error.into_inner(),
            }
        })
    }

    // Recovers the path a flattened field lost by re-checking each group alone.
    fn locate_flat_field_error(input: &RawValue) -> Option<ModelFieldError> {
        let metadata = try_leaf::<MetadataFields>(input);
        metadata
            .or_else(|| try_leaf::<ReasoningCapabilities>(input))
            .or_else(|| try_leaf::<MediaCapabilities>(input))
            .or_else(|| try_leaf::<ToolingCapabilities>(input))
            .or_else(|| try_leaf::<VendorCapabilities>(input))
    }
}

fn try_leaf<T>(input: &RawValue) -> Option<ModelFieldError>
where
    T: DeserializeOwned,
{
    let mut deserializer = serde_json::Deserializer::from_str(input.get());
    let error = serde_path_to_error::deserialize::<_, T>(&mut deserializer).err()?;
    let path = error.path().to_string();
    if path == "." {
        return None;
    }
    Some(ModelFieldError::Json {
        path,
        source: error.into_inner(),
    })
}

mod tests;
