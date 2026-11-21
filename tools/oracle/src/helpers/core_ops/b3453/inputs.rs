use std::sync::Arc;

use betterlite_core::rag::ingestion::config::IngestionOptions;
use betterlite_core::rag::ingestion::{IngestionPorts, IngestionState};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use serde::Deserialize;

use super::fakes::{NoCredentials, NoFetch, NoOcr};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateResponseInput {
    #[serde(default)]
    pub(super) id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoreInput {
    #[serde(default)]
    pub(super) options: IngestionOptions,
    #[serde(default)]
    pub(super) file_content_base64: Option<String>,
    #[serde(default)]
    pub(super) filename: Option<String>,
    #[serde(default)]
    pub(super) content_type: Option<String>,
    #[serde(default)]
    pub(super) existing_file_id: Option<String>,
    #[serde(default)]
    pub(super) create_response: Option<CreateResponseInput>,
    #[serde(default)]
    pub(super) create_error: Option<String>,
    #[serde(default)]
    pub(super) attach_error: Option<String>,
    #[serde(default)]
    pub(super) upload_id: Option<String>,
    #[serde(default)]
    pub(super) upload_error: Option<String>,
}

pub(super) fn build_state(input: &StoreInput) -> Result<IngestionState, Box<HelperError>> {
    IngestionState::new(
        input.options.clone(),
        None,
        IngestionPorts {
            ocr: Arc::new(NoOcr),
            uploader: Arc::new(NoFetch),
            credentials: Arc::new(NoCredentials),
        },
    )
    .or_value_error()
}
