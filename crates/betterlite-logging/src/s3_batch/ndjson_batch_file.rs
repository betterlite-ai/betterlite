use crate::private_json::error::PrivateJsonError;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use crate::s3_batch::Element;
use crate::s3_transport::batch_element::ContentType;
use crate::s3_transport::signing::Clock;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::time::UNIX_EPOCH;
pub const COLD_STORAGE_LOGGER: &str = "s3_v2";
#[must_use]
pub fn batch_file_mode_active(enabled: bool, cold_storage_logger: Option<&str>) -> bool {
    if !enabled {
        return false;
    }
    if cold_storage_logger == Some(COLD_STORAGE_LOGGER) {
        tracing::warn!(
            "s3 logging: s3_batch_file_upload is ignored because s3_v2 is the cold storage logger; per-request objects are required for spend log lookups"
        );
        return false;
    }
    true
}
pub fn batch_file_elements(
    batch: &[Element],
    stamp: &str,
) -> Result<Vec<Element>, PrivateJsonError> {
    let mut individual = Vec::new();
    let mut groups: BTreeMap<&str, Vec<&Element>> = BTreeMap::new();
    for element in batch {
        if element.body.is_some() {
            individual.push(element.clone());
            continue;
        }
        let parent = element
            .s3_object_key
            .rsplit_once('/')
            .map_or("", |(parent, _name)| parent);
        groups.entry(parent).or_default().push(element);
    }
    let mut files = individual;
    for (parent, elements) in groups {
        files.push(build_batch_file(&elements, parent, stamp)?);
    }
    Ok(files)
}
pub fn batch_stamp(clock: &dyn Clock) -> String {
    let elapsed = clock.now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX);
    DateTime::<Utc>::from_timestamp(seconds, elapsed.subsec_nanos()).map_or_else(
        || "00-00-00".to_owned(),
        |moment| moment.format("%H-%M-%S").to_string(),
    )
}
fn build_batch_file(
    elements: &[&Element],
    parent: &str,
    stamp: &str,
) -> Result<Element, PrivateJsonError> {
    let name = format!("batch_{stamp}_{}", uuid::Uuid::new_v4().simple());
    let mut lines = Vec::with_capacity(elements.len());
    for element in elements {
        lines.push(safe_dumps(
            &element.payload,
            DEFAULT_MAX_RECURSE_DEPTH,
            None,
        )?);
    }
    let retrying_since = elements
        .iter()
        .filter_map(|element| element.retrying_since)
        .reduce(f64::min);
    let key = if parent.is_empty() {
        format!("{name}.jsonl")
    } else {
        format!("{parent}/{name}.jsonl")
    };
    Ok(Element {
        payload: Json::Object(IndexMap::new()),
        s3_object_key: key,
        s3_object_download_filename: format!("{name}.jsonl"),
        body: Some(lines.join("\n")),
        content_type: ContentType::Ndjson,
        retrying_since,
    })
}
mod tests;
