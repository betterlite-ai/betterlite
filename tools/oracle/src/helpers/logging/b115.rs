use std::collections::BTreeMap;

use betterlite_logging::s3_audit::element::audit_element;
use betterlite_logging::s3_v2::PartitionGranularityCache;
use betterlite_logging::s3_v2::parameters::resolve_s3_parameters;
use betterlite_logging::s3_v2::source::Source;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(default)]
    global_params: Source<Value>,
    override_params: Option<Source<Value>>,
    record: Value,
    now: DateTime<Utc>,
    #[serde(default)]
    settings: BTreeMap<String, String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let defaults = Source::<Value>::default();
        let parameters = resolve_s3_parameters(
            &defaults,
            &input.global_params,
            input.override_params.as_ref(),
            &input.settings,
        );
        let mut cache = PartitionGranularityCache::default();
        let granularity = cache.resolve(
            parameters.connection.partition_granularity.as_ref(),
            &input.settings,
        );
        let record: Json = serde_json::from_value(input.record.clone())?;
        let element = audit_element(
            parameters.connection.path.as_deref().unwrap_or_default(),
            granularity,
            &record,
            input.now,
        )
        .or_value_error()?;
        Ok(json!({
            "payload": element.payload,
            "s3_object_key": element.s3_object_key,
            "s3_object_download_filename": element.s3_object_download_filename,
        }))
    }
}
