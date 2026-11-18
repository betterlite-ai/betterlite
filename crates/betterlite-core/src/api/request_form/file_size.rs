use std::collections::BTreeSet;
use std::ops::Div as _;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use num_traits::ToPrimitive as _;
use serde_json::Number;
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;

const MEBIBYTE: f64 = 1_048_576.0;
const METADATA_FIELD: &str = "metadata";
const SIZE_FIELD: &str = "file_size_in_mb";
const MODEL_FIELD: &str = "model";

// The answer to Router.get_deployment_by_model_group_name for a transcription model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UploadLimit {
    Absent,
    Configured(f64),
    Failed,
}

pub trait ModelUploadLimits {
    fn upload_limit_mebibytes(&self, model: &str) -> UploadLimit;
}

pub struct UploadSizeCheck<'check> {
    pub request_data: &'check mut JsonObject,
    pub file_size: Option<u64>,
    pub router_names: &'check BTreeSet<String>,
    pub router: Option<&'check dyn ModelUploadLimits>,
    pub limits: &'check ApiLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadSizeOutcome {
    pub proceeded: bool,
    pub lookup_failed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadErrorFacts {
    pub status: u16,
    pub error_type: &'static str,
    pub param: &'static str,
    pub code: &'static str,
}

#[derive(Debug, Error, PartialEq)]
pub enum UploadSizeError {
    #[error("the request has no model for a file size check")]
    MissingModel,
    #[error("the request metadata field is not an object")]
    MetadataNotObject,
    #[error("the file size cannot be represented as a float")]
    Unrepresentable,
    #[error("File size is too large. Passed file size: {size_mb} MB. Max file size: {limit_mb} MB")]
    TooLarge { size_mb: f64, limit_mb: f64 },
    #[error("File size is too large. Passed file size: {size_mb} MB. Max file size: {cap_mb} MB")]
    AboveSafetyCap { size_mb: f64, cap_mb: f64 },
}

impl UploadSizeError {
    #[must_use]
    pub const fn facts(&self) -> UploadErrorFacts {
        match self {
            Self::TooLarge { .. } => UploadErrorFacts {
                status: 400,
                error_type: "bad_request",
                param: "file",
                code: "400",
            },
            Self::AboveSafetyCap { .. } => UploadErrorFacts {
                status: 413,
                error_type: "invalid_request_error",
                param: "file",
                code: "request_too_large",
            },
            Self::MissingModel | Self::MetadataNotObject | Self::Unrepresentable => {
                UploadErrorFacts {
                    status: 400,
                    error_type: "invalid_request_error",
                    param: "request_body",
                    code: "400",
                }
            }
        }
    }
}

// check_file_size_under_limit: the MiB metadata is recorded before any lookup, a model outside router_model_names keeps no model limit, and a lookup error logs and falls through.
pub fn check_file_size_under_limit(
    check: UploadSizeCheck<'_>,
) -> Result<UploadSizeOutcome, UploadSizeError> {
    let UploadSizeCheck {
        request_data,
        file_size,
        router_names,
        router,
        limits,
    } = check;
    let size_mb = file_size
        .unwrap_or(0)
        .to_f64()
        .ok_or(UploadSizeError::Unrepresentable)?
        .div(MEBIBYTE);
    record_size(&mut *request_data, size_mb)?;
    let (configured, lookup_failed) = model_limit(router, router_names, &*request_data)?;
    enforce(size_mb, configured, limits)?;
    Ok(UploadSizeOutcome {
        proceeded: true,
        lookup_failed,
    })
}

fn record_size(request_data: &mut JsonObject, size_mb: f64) -> Result<(), UploadSizeError> {
    let number = Number::from_f64(size_mb).ok_or(UploadSizeError::Unrepresentable)?;
    let entry = request_data
        .entry(METADATA_FIELD.to_owned())
        .or_insert_with(|| Json::Object(JsonObject::new()));
    let Json::Object(metadata) = entry else {
        return Err(UploadSizeError::MetadataNotObject);
    };
    metadata.insert(SIZE_FIELD.to_owned(), Json::Number(number));
    Ok(())
}

fn model_limit(
    router: Option<&dyn ModelUploadLimits>,
    router_names: &BTreeSet<String>,
    request_data: &JsonObject,
) -> Result<(Option<f64>, bool), UploadSizeError> {
    let Some(lookup) = router else {
        return Ok((None, false));
    };
    let Some(Json::Text(model)) = request_data.get(MODEL_FIELD) else {
        return Err(UploadSizeError::MissingModel);
    };
    if !router_names.contains(model) {
        return Ok((None, false));
    }
    match lookup.upload_limit_mebibytes(model) {
        UploadLimit::Absent => Ok((None, false)),
        UploadLimit::Configured(limit) => Ok((Some(limit), false)),
        UploadLimit::Failed => {
            tracing::error!(model = %model, "Got error when checking file size");
            Ok((None, true))
        }
    }
}

fn enforce(
    size_mb: f64,
    configured: Option<f64>,
    limits: &ApiLimits,
) -> Result<(), UploadSizeError> {
    if let Some(limit_mb) = configured {
        if size_mb > limit_mb {
            return Err(UploadSizeError::TooLarge { size_mb, limit_mb });
        }
        return Ok(());
    }
    let cap_mb = cap_mebibytes(limits)?;
    if size_mb > cap_mb {
        return Err(UploadSizeError::AboveSafetyCap { size_mb, cap_mb });
    }
    Ok(())
}

fn cap_mebibytes(limits: &ApiLimits) -> Result<f64, UploadSizeError> {
    let cap = limits
        .max_json_bytes
        .to_u64()
        .ok_or(UploadSizeError::Unrepresentable)?;
    Ok(cap
        .to_f64()
        .ok_or(UploadSizeError::Unrepresentable)?
        .div(MEBIBYTE))
}
