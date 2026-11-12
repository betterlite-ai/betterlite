use betterlite_bedrock::s3_access::{TrustedModelCredentials, TrustedValue, frozen_mapping};
use betterlite_context::cloud_storage;
use betterlite_vertex::vertex_gcs_access::bucket::{
    BucketCandidates, GCS_BATCH_BUCKET_ENV, GCS_BUCKET_ENV, configured_bucket_name,
};
use betterlite_vertex::vertex_gcs_access::decode::decode_vertex_gcs_id;
use betterlite_vertex::vertex_gcs_access::parse::GcsObject;
use serde_json::{Value, json};

use super::input::{Input, Operation, WireValue};
use crate::helpers::HelperResult;

pub(super) const STORAGE_URL_PREFIX: &str = "https://storage.googleapis.com/storage/v1/b/";

pub(super) fn run(input: &Input) -> HelperResult {
    Ok(match input.operation {
        Operation::ConfiguredPrefix
        | Operation::EncodedTraversal
        | Operation::ForeignBucket
        | Operation::TerminalSlash
        | Operation::PercentReencoding
        | Operation::ForgedTrust => parse_values(input, &[]),
        Operation::TrustedLegacy => parse_values(input, input.trusted.as_slice()),
        Operation::FilenameSanitization => filename_values(input),
        Operation::ModelPath => model_path_values(input),
    })
}

fn trusted_record(pairs: &[(String, WireValue)]) -> Option<TrustedModelCredentials> {
    (!pairs.is_empty()).then(|| {
        frozen_mapping(
            pairs
                .iter()
                .map(|(name, value)| (name.clone(), trusted_value(value))),
        )
    })
}

fn trusted_value(value: &WireValue) -> TrustedValue {
    match value {
        WireValue::Text(text) => TrustedValue::Text(text.clone()),
        WireValue::Boolean(flag) => TrustedValue::Boolean(*flag),
        WireValue::Null(()) => TrustedValue::Null,
        WireValue::Other(_) => TrustedValue::Other,
    }
}

fn bucket_candidates(input: &Input) -> BucketCandidates {
    BucketCandidates {
        deployment: input.params.get("gcs_bucket_name").cloned(),
        deployment_alias: input.params.get("bucket_name").cloned(),
        batch_environment: input.environment.get(GCS_BATCH_BUCKET_ENV).cloned(),
        environment: input.environment.get(GCS_BUCKET_ENV).cloned(),
    }
}

fn parse_values(input: &Input, trusted_pairs: &[(String, WireValue)]) -> Value {
    let trusted = trusted_record(trusted_pairs);
    let configured = configured_bucket_name(&bucket_candidates(input));
    let results: Vec<Value> = input
        .file_ids
        .iter()
        .map(|file_id| match (&configured, &trusted) {
            (Ok(bucket), trusted) => decoded_value(file_id, bucket, trusted.as_ref()),
            (Err(_error), _trusted) => json!({"failed": true}),
        })
        .collect();
    json!({"results": results})
}

fn decoded_value(
    file_id: &str,
    configured_bucket_name: &str,
    trusted: Option<&TrustedModelCredentials>,
) -> Value {
    decode_vertex_gcs_id(file_id.as_bytes(), configured_bucket_name, trusted).map_or_else(
        |_error| json!({"failed": true}),
        |object| json!({"url": storage_url(&object)}),
    )
}

fn storage_url(object: &GcsObject) -> String {
    format!(
        "{STORAGE_URL_PREFIX}{}/o/{}",
        object.bucket, object.encoded_object
    )
}

fn filename_values(input: &Input) -> Value {
    let components: Vec<String> = input
        .filenames
        .iter()
        .map(|filename| cloud_storage::sanitize_component(filename.as_deref(), "file"))
        .collect();
    json!({"components": components})
}

fn model_path_values(input: &Input) -> Value {
    let paths: Vec<String> = input
        .model_paths
        .iter()
        .map(|path| cloud_storage::sanitize_path(path.as_deref(), "model"))
        .collect();
    json!({"paths": paths})
}
