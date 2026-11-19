use std::collections::BTreeSet;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_form::file_size::{UploadSizeCheck, check_file_size_under_limit};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use serde_json::json;

use super::{Input, failure};

const HELPER_MODEL: &str = "whisper";

pub(super) fn upload_size(input: &Input) -> HelperResult {
    let mut request_data = JsonObject::new();
    request_data.insert("model".to_owned(), Json::Text(HELPER_MODEL.to_owned()));
    if !input.existing_metadata.is_empty() {
        request_data.insert(
            "metadata".to_owned(),
            Json::Object(input.existing_metadata.clone().into_iter().collect()),
        );
    }
    let outcome = check_file_size_under_limit(UploadSizeCheck {
        request_data: &mut request_data,
        file_size: input.file_size,
        router_names: &BTreeSet::new(),
        router: None,
        limits: &ApiLimits::default(),
    })
    .map_err(|error| failure("ValueError", error.to_string()))?;
    let Some(metadata) = request_data.get("metadata") else {
        return Err(failure("KeyError", "metadata".to_owned()));
    };
    let metadata_value = serde_json::to_value(metadata).or_value_error()?;
    Ok(json!({"proceeded": outcome.proceeded, "metadata": metadata_value}))
}
