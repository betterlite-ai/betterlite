use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::plan::strip_version_suffix;

// LiteLLM's request_data: model and messages are read by the handler and every other key is sent as it arrived
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PartnerCountBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<Json>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Json>,
}

impl PartnerCountBody {
    // LiteLLM rebuilds request_data with the stripped model and leaves every other key untouched
    #[must_use]
    pub fn stripped_model(&self) -> Self {
        Self {
            model: self
                .model
                .as_deref()
                .map(strip_version_suffix)
                .map(str::to_owned),
            messages: self.messages.clone(),
            extra: self.extra.clone(),
        }
    }
}
