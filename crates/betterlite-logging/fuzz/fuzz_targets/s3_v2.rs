#![no_main]

use betterlite_logging::s3_v2::PartitionGranularityCache;
use betterlite_logging::s3_v2::events::{Event, create_batch_element};
use betterlite_logging::s3_v2::parameters::resolve_s3_parameters;
use betterlite_logging::s3_v2::source::Source;
use chrono::NaiveDateTime;
use libfuzzer_sys::fuzz_target;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 65_536 {
        return;
    }
    if let Ok((source, payload)) =
        serde_json::from_slice::<(Source<Box<RawValue>>, Option<Box<RawValue>>)>(bytes)
    {
        let parameters = resolve_s3_parameters(
            &Source::default(),
            &source,
            None,
            &BTreeMap::<String, String>::new(),
        );
        let _result = create_batch_element(
            &parameters,
            &Event {
                start_time: NaiveDateTime::default(),
                payload: payload.as_ref(),
                cold_storage_enabled: true,
            },
            &mut PartitionGranularityCache::default(),
            &BTreeMap::<String, String>::new(),
        );
    }
});
