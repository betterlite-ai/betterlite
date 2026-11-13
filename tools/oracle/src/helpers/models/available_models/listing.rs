use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_models::available_models::listing::{ListingCredentials, ModelListing};
use betterlite_models::provider_metadata_factory::selection::ProviderModelMetadata;
use secrecy::ExposeSecret;
use serde_json::{Value, json};

use super::input::ListingOutcome;

#[derive(Debug, thiserror::Error)]
#[error("the case scripts no working model listing for {class}")]
pub(super) struct ListingFailure {
    class: &'static str,
}

pub(super) struct ScriptedListing {
    pub outcomes: BTreeMap<String, ListingOutcome>,
    pub records: Mutex<Vec<Value>>,
}

impl ModelListing for ScriptedListing {
    type Error = ListingFailure;

    fn get_models(
        &self,
        adapter: &ProviderModelMetadata,
        credentials: ListingCredentials<'_>,
    ) -> Result<Vec<String>, ListingFailure> {
        let class = adapter.class_name();
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "class": class,
                "api_key": credentials.api_key.map(ExposeSecret::expose_secret),
                "api_base": credentials.api_base,
            }));
        match self.outcomes.get(class) {
            Some(ListingOutcome::Models(models)) => Ok(models.clone()),
            Some(ListingOutcome::Error(_message)) => Err(ListingFailure { class }),
            None => Err(ListingFailure { class }),
        }
    }
}
