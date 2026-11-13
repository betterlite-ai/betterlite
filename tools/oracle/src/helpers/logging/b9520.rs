use std::collections::BTreeMap;

use betterlite_logging::cold_storage_key::{ColdStorageTarget, stamp_cold_storage_object_key};
use betterlite_logging::logging_metadata::params::MetadataMap;
use betterlite_logging::logging_metadata::setup::{
    StandardMetadataRequest, get_standard_logging_metadata,
};
use betterlite_logging::s3_transport::option_value::OptionValue;
use betterlite_logging::s3_v2::PartitionGranularityCache;
use chrono::NaiveDateTime;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    cold_storage_custom_logger: Option<String>,
    metadata: Option<MetadataMap>,
    start_time: Option<NaiveDateTime>,
    response_id: Option<String>,
    s3_v2: Option<S3V2Logger>,
    #[serde(default)]
    settings: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub(crate) struct S3V2Logger {
    path: Option<String>,
    partition_granularity: Option<OptionValue>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let target = cold_storage_target(input);
        let request = StandardMetadataRequest {
            metadata: input.metadata.as_ref(),
            params: None,
            prompt_integration: None,
            applied_guardrails: None,
            mcp_tool_call_metadata: None,
            vector_store_request_metadata: None,
            usage_object: None,
            custom_llm_provider: None,
        };
        let mut record = get_standard_logging_metadata(&request).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        stamp_cold_storage_object_key(
            &mut record,
            input.start_time,
            input.response_id.as_deref(),
            target.as_ref(),
        );
        Ok(json!({"cold_storage_object_key": record.cold_storage_object_key}))
    }
}

fn cold_storage_target(input: &Input) -> Option<ColdStorageTarget> {
    let name = input.cold_storage_custom_logger.as_deref()?;
    if name != "s3_v2" {
        return Some(ColdStorageTarget::default());
    }
    let Some(logger) = input.s3_v2.as_ref() else {
        return Some(ColdStorageTarget::default());
    };
    let mut cache = PartitionGranularityCache::default();
    Some(ColdStorageTarget {
        s3_path: logger.path.clone().unwrap_or_default(),
        partition_granularity: cache
            .resolve(logger.partition_granularity.as_ref(), &input.settings),
    })
}
