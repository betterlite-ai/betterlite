use betterlite_models::identity::provider_id::ProviderId;

use super::selection::{ChatConfigError, ChatConfigRequest};
use super::tags::{ChatConfigTag, ConfigConstruction, ConfigSelection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonProviderLookup {
    Absent,
    MissingRecord,
    Record,
}

pub trait JsonProviderProjection {
    #[must_use]
    fn lookup(&self, provider: ProviderId) -> JsonProviderLookup;
}

pub fn select_json_provider(
    request: &ChatConfigRequest<'_>,
) -> Result<Option<ConfigSelection<ChatConfigTag>>, ChatConfigError> {
    match request.json_providers.lookup(request.provider) {
        JsonProviderLookup::Absent => Ok(None),
        JsonProviderLookup::MissingRecord => Err(ChatConfigError::MissingJsonRecord {
            provider: request.provider,
        }),
        JsonProviderLookup::Record => Ok(Some(ConfigSelection {
            tag: ChatConfigTag::JsonProvider,
            construction: ConfigConstruction::New(Vec::new()),
        })),
    }
}
