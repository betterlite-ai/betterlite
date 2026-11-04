use betterlite_openai::api::vector_store_records::records::basic::VectorStoreToolParams;
use betterlite_openai::api::vector_store_records::records::vector::{
    BaseVectorStoreAuthCredentials, LiteLlmVectorStoreConfig,
};
use betterlite_openai::api::vector_store_records::wire::{
    VectorStoreRecordKind, WireDecodeLimits, decode_vector_store_records,
};
use serde::Deserialize;
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

const MANAGEMENT: &[(&str, VectorStoreRecordKind)] = &[
    ("update", VectorStoreRecordKind::UpdateRequest),
    ("delete", VectorStoreRecordKind::DeleteRequest),
    ("info", VectorStoreRecordKind::InfoRequest),
    ("managed_index", VectorStoreRecordKind::ManagedIndex),
    ("index_list", VectorStoreRecordKind::IndexListResponse),
    ("index_create", VectorStoreRecordKind::IndexCreateRequest),
];
const CREATE_AND_SEARCH: &[(&str, VectorStoreRecordKind)] = &[
    ("create", VectorStoreRecordKind::CreateRequest),
    ("auto_chunking", VectorStoreRecordKind::CreateRequest),
    ("omitted_static", VectorStoreRecordKind::CreateRequest),
    ("response", VectorStoreRecordKind::CreateResponse),
    ("search", VectorStoreRecordKind::SearchResponse),
    ("failure", VectorStoreRecordKind::SearchFailure),
];
const KNOWLEDGE_BASE: &[(&str, VectorStoreRecordKind)] = &[
    ("request", VectorStoreRecordKind::BedrockRequest),
    ("response", VectorStoreRecordKind::BedrockResponse),
];
const VECTOR_RECORDS: &[(&str, VectorStoreRecordKind)] = &[
    ("managed", VectorStoreRecordKind::ManagedStore),
    ("managed_list", VectorStoreRecordKind::ManagedStoreList),
    ("endpoints", VectorStoreRecordKind::IndexEndpoints),
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
    fields: &[(&str, VectorStoreRecordKind)],
) -> crate::helpers::HelperResult {
    let mut output = source.clone();
    for (name, kind) in fields {
        if let Some(record) = source.get(*name) {
            output.insert((*name).to_owned(), decode_model(*kind, record)?);
        }
    }
    serde_json::to_value(output).map_err(|error| helper_error(error.to_string()))
}

fn round_trip_records(source: &BTreeMap<String, Value>) -> crate::helpers::HelperResult {
    let mut output = decode_group(source, VECTOR_RECORDS)?;
    if let Some(record) = output.get_mut("config") {
        let typed: LiteLlmVectorStoreConfig = serde_json::from_value(record.clone())
            .map_err(|error| helper_error(error.to_string()))?;
        *record = serde_json::to_value(typed).map_err(|error| helper_error(error.to_string()))?;
    }
    if let Some(record) = output.get_mut("auth") {
        let typed: BaseVectorStoreAuthCredentials = serde_json::from_value(record.clone())
            .map_err(|error| helper_error(error.to_string()))?;
        *record = serde_json::to_value(typed).map_err(|error| helper_error(error.to_string()))?;
    }
    Ok(output)
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
            Input::VectorRecordsRoundTrip { record } => round_trip_records(record),
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
