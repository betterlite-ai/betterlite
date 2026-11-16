use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::response_id_traversal::record::{DictRecord, Record};

use super::{ContainerIdentity, DEFAULT_PROVIDER};

const HIDDEN_PARAMS_FIELD: &str = "_hidden_params";

// LiteLLM's _dump_response: a mapping keeps every key, a typed object dumps its public fields only
pub(crate) fn dump_response(response: &Record) -> IndexMap<String, Json> {
    match response {
        Record::Dict(dict) => dict
            .values
            .iter()
            .map(|(name, value)| (name.clone(), record_json(value)))
            .collect(),
        Record::Object { object } => object
            .fields
            .iter()
            .filter(|(name, _)| name.as_str() != HIDDEN_PARAMS_FIELD)
            .map(|(name, value)| (name.clone(), record_json(value)))
            .collect(),
        Record::List(_) | Record::Scalar(_) => IndexMap::from([("id".to_owned(), Json::Null)]),
    }
}

fn record_json(record: &Record) -> Json {
    match record {
        Record::Dict(dict) => Json::Object(
            dict.values
                .iter()
                .map(|(name, value)| (name.clone(), record_json(value)))
                .collect(),
        ),
        Record::Object { object } => Json::Object(
            object
                .fields
                .iter()
                .map(|(name, value)| (name.clone(), record_json(value)))
                .collect(),
        ),
        Record::List(items) => Json::List(items.iter().map(record_json).collect()),
        Record::Scalar(value) => value.clone(),
    }
}

// _custom_llm_provider_from_responses_response; a non-mapping _hidden_params reads as absent here
#[must_use]
pub(crate) fn response_provider(response: &Record) -> String {
    response
        .field(HIDDEN_PARAMS_FIELD)
        .and_then(|hidden| hidden.field("custom_llm_provider"))
        .and_then(Record::as_text)
        .filter(|provider| !provider.is_empty())
        .map_or_else(|| DEFAULT_PROVIDER.to_owned(), str::to_owned)
}

// the file_object a row stores: the dumped response with the resolved provider and upstream id stamped on
#[must_use]
pub(crate) fn owner_file_object(response: &Record, identity: &ContainerIdentity) -> Json {
    let mut fields = dump_response(response);
    fields.insert(
        "custom_llm_provider".to_owned(),
        Json::Text(identity.custom_llm_provider.clone()),
    );
    fields.insert(
        "provider_container_id".to_owned(),
        Json::Text(identity.original_container_id.clone()),
    );
    Json::Object(fields)
}

// the response one container of a responses output is recorded with: {"id": container_id, "object": "container"}
#[must_use]
pub(crate) fn synthetic_container(container_id: &str) -> Record {
    let mut values = IndexMap::new();
    values.insert(
        "id".to_owned(),
        Record::Scalar(Json::Text(container_id.to_owned())),
    );
    values.insert(
        "object".to_owned(),
        Record::Scalar(Json::Text("container".to_owned())),
    );
    Record::Dict(DictRecord { values })
}
