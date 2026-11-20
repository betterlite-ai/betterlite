use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_destination::parameters::tags_from_request_body;
use indexmap::IndexSet;

mod tests;

// dict.fromkeys keeps the first occurrence, so a tag repeated later never moves its descriptor
#[must_use]
pub fn deduplicated_tags(data: &RequestData) -> Vec<String> {
    let mut ordered = IndexSet::new();
    ordered.extend(tags_from_request_body(data));
    ordered.into_iter().collect()
}
