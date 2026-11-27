use std::collections::BTreeMap;

use crate::request_policy::{Params, RequestValue};
use crate::s3_access::{
    S3AccessError, S3AccessParams, configured_bucket_name, configured_bucket_names,
};

use super::{LIST_FILES_LOCATION_PARAM, LIST_FILES_PURPOSE_PARAM};

// os.path.commonprefix over BEDROCK_MANAGED_S3_PREFIXES and the batch pair
pub const ANY_MANAGED_LISTING_PREFIX: &str = "litellm-b";
pub const BATCH_LISTING_PREFIX: &str = "litellm-bedrock-files";
pub const BATCH_OUTPUT_LISTING_PREFIX: &str = "litellm-batch-outputs/";

#[must_use]
pub fn managed_listing_prefix(configured_prefix: &str, purpose: Option<&str>) -> Option<String> {
    let managed = match purpose {
        Some("") | None => ANY_MANAGED_LISTING_PREFIX,
        Some("batch") => BATCH_LISTING_PREFIX,
        Some("batch_output") => BATCH_OUTPUT_LISTING_PREFIX,
        Some(_) => return None,
    };
    if configured_prefix.is_empty() {
        return Some(managed.to_owned());
    }
    Some(format!("{configured_prefix}/{managed}"))
}

#[must_use]
pub fn listing_query(configured_prefix: &str, purpose: Option<&str>) -> Vec<(String, String)> {
    let Some(prefix) = managed_listing_prefix(configured_prefix, purpose) else {
        return vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("max-keys".to_owned(), "0".to_owned()),
        ];
    };
    vec![
        ("list-type".to_owned(), "2".to_owned()),
        ("prefix".to_owned(), prefix),
    ]
}

// the listing walk records what it asked for; only a string counts, as LiteLLM's isinstance does
#[must_use]
pub fn setting_text(params: &Params, name: &str) -> Option<String> {
    match params.get(name) {
        Some(RequestValue::Text(text)) => Some(text.clone()),
        Some(RequestValue::Raw(raw)) => serde_json::from_str::<Option<String>>(raw).ok().flatten(),
        Some(RequestValue::Secret(_) | RequestValue::Mapping(_)) | None => None,
    }
}

#[must_use]
pub fn walked_listing_purpose(params: &Params) -> Option<String> {
    setting_text(params, LIST_FILES_LOCATION_PARAM)
        .or_else(|| setting_text(params, LIST_FILES_PURPOSE_PARAM))
}

pub fn listing_bucket_name(
    access: &S3AccessParams,
    environment: &BTreeMap<String, String>,
    purpose: Option<&str>,
) -> Result<String, S3AccessError> {
    if purpose != Some("batch_output") {
        return configured_bucket_name(access, environment);
    }
    let names = configured_bucket_names(access, environment)?;
    match configured_bucket_name(access, environment) {
        Ok(input) => Ok(names.get(1).cloned().unwrap_or(input)),
        Err(error) => match names.first() {
            Some(name) => Ok(name.clone()),
            None => Err(error),
        },
    }
}

pub fn output_location_still_unlisted(
    access: &S3AccessParams,
    environment: &BTreeMap<String, String>,
    litellm_params: &Params,
) -> Result<bool, S3AccessError> {
    if walked_listing_purpose(litellm_params).is_some() {
        return Ok(false);
    }
    let output = listing_bucket_name(access, environment, Some("batch_output"))?;
    let input = listing_bucket_name(access, environment, None)?;
    Ok(output != input)
}
