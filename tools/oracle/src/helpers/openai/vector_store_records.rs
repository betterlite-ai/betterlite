use betterlite_openai::api::vector_store_records::records::basic::{
    GuardrailAction, VectorStoreIndexType, VectorStoreIntegration, VectorStoreSearchFailureMode,
    VectorStoreStatus, VectorStoreToolParams,
};
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, LiteLlmVectorStoreConfig, VectorStoreFileCounts,
};
use betterlite_openai::api::vector_store_records::wire::{
    VectorStoreRecordKind, WireDecodeLimits, decode_vector_store_records,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum Input {
    ToolParamsOmitOnlyNull {
        #[serde(flatten)]
        params: VectorStoreToolParams,
    },
    ManagementRecordDefaults {
        records: BTreeMap<String, Value>,
    },
    InvalidIndexListLiteral {
        record: Value,
    },
    InvalidMissingId {
        record: Value,
    },
    InvalidIndexInfo {
        record: Value,
    },
    ManagedIndexTimestampForms {
        records: Vec<Value>,
    },
    DiscoveryExtraBodyFields {
        extra_body: Value,
    },
    CreateAndSearchRecords {
        record: BTreeMap<String, Value>,
    },
    KnowledgeBaseRecords {
        record: BTreeMap<String, Value>,
    },
    VectorRecordsRoundTrip {
        record: BTreeMap<String, Value>,
    },
}

pub(crate) struct Op;

use EntryKind::Record;

enum EntryKind {
    Record(VectorStoreRecordKind),
    Config,
    Auth,
    Counts,
    Statuses,
    FailureModes,
    Guardrail,
    Integrations,
    IndexTypes,
}

const MANAGEMENT: &[(&str, EntryKind)] = &[
    ("update", Record(VectorStoreRecordKind::UpdateRequest)),
    ("delete", Record(VectorStoreRecordKind::DeleteRequest)),
    ("info", Record(VectorStoreRecordKind::InfoRequest)),
    ("managed_index", Record(VectorStoreRecordKind::ManagedIndex)),
    (
        "index_list",
        Record(VectorStoreRecordKind::IndexListResponse),
    ),
    (
        "index_create",
        Record(VectorStoreRecordKind::IndexCreateRequest),
    ),
];
const CREATE_AND_SEARCH: &[(&str, EntryKind)] = &[
    ("create", Record(VectorStoreRecordKind::CreateRequest)),
    (
        "auto_chunking",
        Record(VectorStoreRecordKind::CreateRequest),
    ),
    (
        "omitted_static",
        Record(VectorStoreRecordKind::CreateRequest),
    ),
    ("response", Record(VectorStoreRecordKind::CreateResponse)),
    ("search", Record(VectorStoreRecordKind::SearchResponse)),
    ("failure", Record(VectorStoreRecordKind::SearchFailure)),
    ("counts", EntryKind::Counts),
    ("response_statuses", EntryKind::Statuses),
    ("failure_modes", EntryKind::FailureModes),
];
const KNOWLEDGE_BASE: &[(&str, EntryKind)] = &[
    ("request", Record(VectorStoreRecordKind::BedrockRequest)),
    ("response", Record(VectorStoreRecordKind::BedrockResponse)),
    (
        "response_empty_nulls",
        Record(VectorStoreRecordKind::BedrockResponse),
    ),
    ("response_guardrail", EntryKind::Guardrail),
];
const VECTOR_RECORDS: &[(&str, EntryKind)] = &[
    ("managed", Record(VectorStoreRecordKind::ManagedStore)),
    (
        "managed_list",
        Record(VectorStoreRecordKind::ManagedStoreList),
    ),
    ("endpoints", Record(VectorStoreRecordKind::IndexEndpoints)),
    ("config", EntryKind::Config),
    ("auth", EntryKind::Auth),
    ("integrations", EntryKind::Integrations),
    ("index_types", EntryKind::IndexTypes),
];

fn decode_model(kind: VectorStoreRecordKind, record: &Value) -> crate::helpers::HelperResult {
    let bytes = serde_json::to_vec(record).map_err(|error| helper_error(error.to_string()))?;
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    };
    let decoded = decode_vector_store_records(kind, &bytes, limits)
        .map_err(|error| helper_error(error.to_string()))?;
    serde_json::to_value(decoded).map_err(|error| helper_error(error.to_string()))
}

fn decode_group(
    source: &BTreeMap<String, Value>,
    fields: &[(&str, EntryKind)],
) -> crate::helpers::HelperResult {
    let mut output = BTreeMap::new();
    for (name, value) in source {
        let (_, kind) = fields
            .iter()
            .find(|(field, _)| *field == name.as_str())
            .ok_or_else(|| helper_error(format!("unmapped case entry `{name}`")))?;
        output.insert(name.clone(), decode_entry(kind, value)?);
    }
    serde_json::to_value(output).map_err(|error| helper_error(error.to_string()))
}

fn decode_entry(kind: &EntryKind, value: &Value) -> crate::helpers::HelperResult {
    match kind {
        Record(record_kind) => decode_model(*record_kind, value),
        EntryKind::Config => round_trip::<LiteLlmVectorStoreConfig>(value),
        EntryKind::Auth => round_trip::<BaseVectorStoreAuthCredentials>(value),
        EntryKind::Counts => round_trip::<VectorStoreFileCounts>(value),
        EntryKind::Statuses => round_trip::<Vec<VectorStoreStatus>>(value),
        EntryKind::FailureModes => round_trip::<Vec<VectorStoreSearchFailureMode>>(value),
        EntryKind::Guardrail => round_trip::<GuardrailAction>(value),
        EntryKind::Integrations => round_trip::<Vec<VectorStoreIntegration>>(value),
        EntryKind::IndexTypes => round_trip::<Vec<VectorStoreIndexType>>(value),
    }
}

fn round_trip<T>(value: &Value) -> crate::helpers::HelperResult
where
    T: DeserializeOwned + Serialize,
{
    let typed: T =
        serde_json::from_value(value.clone()).map_err(|error| helper_error(error.to_string()))?;
    serde_json::to_value(typed).map_err(|error| helper_error(error.to_string()))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::ToolParamsOmitOnlyNull { params } => serde_json::to_value(params.to_dict())
                .map_err(|error| helper_error(error.to_string())),
            Input::ManagedIndexTimestampForms { records } => records
                .iter()
                .map(|record| decode_model(VectorStoreRecordKind::ManagedIndex, record))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Input::InvalidIndexListLiteral { record } => {
                decode_model(VectorStoreRecordKind::IndexListResponse, record)
            }
            Input::InvalidMissingId { record } => {
                decode_model(VectorStoreRecordKind::DeleteRequest, record)
            }
            Input::InvalidIndexInfo { record } => {
                decode_model(VectorStoreRecordKind::IndexCreateRequest, record)
            }
            Input::DiscoveryExtraBodyFields { extra_body } => {
                decode_model(VectorStoreRecordKind::DiscoveryEngine, extra_body)
            }
            Input::ManagementRecordDefaults { records } => decode_group(records, MANAGEMENT),
            Input::CreateAndSearchRecords { record } => decode_group(record, CREATE_AND_SEARCH),
            Input::KnowledgeBaseRecords { record } => decode_group(record, KNOWLEDGE_BASE),
            Input::VectorRecordsRoundTrip { record } => decode_group(record, VECTOR_RECORDS),
        }
    }
}

fn helper_error(message: String) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: String::from("ValidationError"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
