use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::helpers::{Helper, HelperResult};

mod fixtures;
mod operations;
mod tests;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    ManagedFileIdDecode,
    ConfiguredInputOutputBuckets,
    ConfiguredPrefixBoundary,
    ForeignBucketRejected,
    LegacyIdOptIn,
    RequestCredentialsRejected,
    KmsOwnerPrecedence,
    S3ModelHelpers,
    PlainS3UriParser,
    FilesHandlerInitialization,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    file_ids: Vec<String>,
    #[serde(default)]
    trusted: Option<Vec<(String, WireValue)>>,
    // a plain JSON mapping never becomes the trusted snapshot, so this input only has to decode
    #[serde(rename = "plain", default)]
    _plain: Option<Vec<(String, WireValue)>>,
    #[serde(default)]
    malformed: Option<Vec<(String, WireValue)>>,
    #[serde(default)]
    request_bucket_name: Option<String>,
    #[serde(default)]
    request_output_bucket_name: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    s3_uri: Option<String>,
    #[serde(default)]
    buckets: Vec<String>,
    #[serde(default)]
    legacy: bool,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    litellm_params: BTreeMap<String, WireValue>,
    #[serde(default)]
    optional_params: Option<BTreeMap<String, WireValue>>,
    #[serde(default)]
    models: Vec<String>,
    #[serde(default)]
    uris: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum WireValue {
    Text(String),
    Boolean(bool),
    Null(()),
    Other(IgnoredAny),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        operations::run(input)
    }
}
