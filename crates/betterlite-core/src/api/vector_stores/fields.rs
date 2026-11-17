use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::vector_stores::error::VectorStoresError;
use crate::api::vector_stores::{
    CHOICES, LOGGING_OBJECT, MODEL_CALL_DETAILS, PROVIDER_FIELDS, SEARCH_FAILURES, SEARCH_RESULTS,
};

// The sequences the pre-call step collected, borrowed from whichever record holds them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetrievalFields<'data> {
    pub search_results: &'data [Json],
    pub failures: &'data [Json],
}

// The success hook reads model_call_details, not request_data; Ok(None) means there was no logging object (vector_store_pre_call_hook.py:431-442).
pub fn from_call_details(
    request_data: &IndexMap<String, Json>,
) -> Result<Option<RetrievalFields<'_>>, VectorStoresError> {
    match request_data.get(LOGGING_OBJECT) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(logging)) => match logging.get(MODEL_CALL_DETAILS) {
            None | Some(Json::Null) => Ok(Some(RetrievalFields::default())),
            Some(Json::Object(details)) => Ok(Some(RetrievalFields {
                search_results: sequence(details, SEARCH_RESULTS)?,
                failures: sequence(details, SEARCH_FAILURES)?,
            })),
            Some(_) => Err(VectorStoresError::CallDetailsNotObject),
        },
        Some(_) => Err(VectorStoresError::LoggingNotObject),
    }
}

// The streaming hook reads the two fields straight out of request_data (vector_store_pre_call_hook.py:488-489).
pub fn from_request_data(
    request_data: &IndexMap<String, Json>,
) -> Result<RetrievalFields<'_>, VectorStoresError> {
    Ok(RetrievalFields {
        search_results: sequence(request_data, SEARCH_RESULTS)?,
        failures: sequence(request_data, SEARCH_FAILURES)?,
    })
}

fn sequence<'data>(
    record: &'data IndexMap<String, Json>,
    field: &'static str,
) -> Result<&'data [Json], VectorStoresError> {
    match record.get(field) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::List(items)) => Ok(items),
        Some(_) => Err(VectorStoresError::FieldNotList { field }),
    }
}

// Both hooks write into each choice's message or delta, keeping existing fields and skipping targets that are absent or null (vector_store_pre_call_hook.py:454-462).
pub fn attach_to_choices(
    record: &mut IndexMap<String, Json>,
    member: &'static str,
    fields: RetrievalFields<'_>,
) -> Result<(), VectorStoresError> {
    let Some(Json::List(choices)) = record.get_mut(CHOICES) else {
        return Ok(());
    };
    for (index, choice) in choices.iter_mut().enumerate() {
        attach(choice, member, index, fields)?;
    }
    Ok(())
}

fn attach(
    choice: &mut Json,
    member: &'static str,
    index: usize,
    fields: RetrievalFields<'_>,
) -> Result<(), VectorStoresError> {
    let Json::Object(choice_map) = choice else {
        return Ok(());
    };
    let Some(target) = choice_map.get_mut(member) else {
        return Ok(());
    };
    if matches!(*target, Json::Null) {
        return Ok(());
    }
    let Json::Object(member_object) = target else {
        return Err(VectorStoresError::TargetNotObject { index, member });
    };
    let mut provider_fields = match member_object.get(PROVIDER_FIELDS) {
        None | Some(Json::Null) => IndexMap::new(),
        Some(Json::Object(existing)) => existing.clone(),
        Some(_) => return Err(VectorStoresError::ProviderFieldsNotObject { index }),
    };
    if !fields.search_results.is_empty() {
        provider_fields.insert(
            SEARCH_RESULTS.to_owned(),
            Json::List(fields.search_results.to_vec()),
        );
    }
    if !fields.failures.is_empty() {
        provider_fields.insert(
            SEARCH_FAILURES.to_owned(),
            Json::List(fields.failures.to_vec()),
        );
    }
    member_object.insert(PROVIDER_FIELDS.to_owned(), Json::Object(provider_fields));
    Ok(())
}
