use chrono::NaiveDateTime;

use crate::fields::payload::records::StandardLoggingMetadata;
use crate::s3_transport::naming::object_key;
use crate::s3_transport::options::PartitionGranularity;

mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColdStorageTarget {
    pub s3_path: String,
    pub partition_granularity: PartitionGranularity,
}

impl Default for ColdStorageTarget {
    fn default() -> Self {
        Self {
            s3_path: String::new(),
            partition_granularity: PartitionGranularity::Day,
        }
    }
}

pub trait ColdStorageLocator: Send + Sync {
    fn locate(&self) -> Option<ColdStorageTarget>;
}

pub fn stamp_cold_storage_object_key(
    metadata: &mut StandardLoggingMetadata,
    start_time: Option<NaiveDateTime>,
    response_id: Option<&str>,
    target: Option<&ColdStorageTarget>,
) {
    let (Some(started_at), Some(response_identifier), Some(destination)) =
        (start_time, response_id, target)
    else {
        return;
    };
    let file_name = format!(
        "time-{}_{response_identifier}",
        started_at.format("%H-%M-%S-%6f")
    );
    metadata.cold_storage_object_key = Some(object_key(
        &destination.s3_path,
        "",
        started_at,
        &file_name,
        destination.partition_granularity,
    ));
}
