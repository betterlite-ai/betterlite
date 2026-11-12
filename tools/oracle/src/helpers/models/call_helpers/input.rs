use betterlite_context::request_records::record::RequestRecord;
use betterlite_models::call_helpers::expected_error::ClientErrorFacts;
use betterlite_models::call_helpers::headers::{AdditionalHeaders, RequestParams};
use betterlite_models::call_helpers::metadata::{CallParams, MetadataMap, MetadataSlots};
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    FinishReasons {
        values: Vec<String>,
    },
    TokenLimits {
        values: Vec<Json>,
    },
    ExpectedClientError {
        values: Vec<Option<ClientErrorFacts>>,
    },
    MetadataBucket {
        steps: Vec<MetadataSlots>,
    },
    TrustedFlags {
        steps: Vec<TrustedStep>,
    },
    ModelReconstruction {
        values: Vec<ModelNameInput>,
    },
    ResponseHeaders {
        steps: Vec<HeaderStep>,
    },
    UpstreamInternalHeader {
        upstream: IndexMap<String, String>,
        trusted: AdditionalHeaders,
    },
    SnapshotIsolation {
        record: RequestRecord,
    },
    FilterInternal {
        record: RequestRecord,
        additional: Vec<String>,
    },
    DropEnvInvalid {
        values: Vec<Option<String>>,
    },
    DropRequestInvalid {
        values: Vec<Json>,
    },
    SafeDivide {
        values: Vec<SafeDivideInput>,
    },
    SafeDivideSeconds {
        values: Vec<SafeDivideSecondsInput>,
    },
    RequestHeaders {
        steps: Vec<Option<RequestParams>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum TrustedStep {
    LitellmMetadata {
        litellm_params: Option<CallParams>,
    },
    StampedOauthToken {
        metadata: Option<MetadataMap>,
        litellm_params: Option<CallParams>,
    },
    AddMissingSpendMetadata {
        litellm_metadata: MetadataMap,
        metadata: MetadataMap,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelNameInput {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default)]
    pub(crate) deployment: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum HeaderStep {
    Project {
        headers: IndexMap<String, String>,
    },
    SetHidden {
        headers: IndexMap<String, String>,
        existing: AdditionalHeaders,
    },
    GetHidden {
        hidden: Option<Json>,
    },
    StreamAttributes {
        response: IndexMap<String, Json>,
        original_chunk: IndexMap<String, Json>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SafeDivideInput {
    pub(crate) numerator: Decimal,
    pub(crate) denominator: Decimal,
    pub(crate) default: Decimal,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SafeDivideSecondsInput {
    pub(crate) seconds: Decimal,
    pub(crate) denominator: Decimal,
    #[serde(default)]
    pub(crate) default: Option<Decimal>,
}
