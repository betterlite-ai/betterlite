use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::FileParams;
use betterlite_openai::openai_base_config::metadata_filter_limit;
use indexmap::IndexMap;

use super::error::FileTransportError;
use super::service::FileCall;

// Create logs and sends its transformed body, list its transformed query; both take the overlay before the nulls go
pub(crate) fn fields(call: &FileCall<'_>) -> Result<IndexMap<String, Json>, FileTransportError> {
    match call {
        FileCall::Create(create) => {
            let mut merged = merged(create.create_request, create.extra_body)?;
            clean_attributes(&mut merged)?;
            Ok(merged)
        }
        FileCall::List(list) => merged(list.query_params, list.extra_query),
    }
}

// dict(params) keeps an explicit null, the overlay replaces whole keys, and _clean_dict then drops every null
fn merged(
    params: &FileParams,
    extra: Option<&IndexMap<String, Json>>,
) -> Result<IndexMap<String, Json>, FileTransportError> {
    let encoded = serde_json::to_value(params).map_err(FileTransportError::Encode)?;
    let mut fields: IndexMap<String, Json> =
        serde_json::from_value(encoded).map_err(FileTransportError::Encode)?;
    for (name, value) in extra.into_iter().flatten() {
        fields.insert(name.clone(), value.clone());
    }
    fields.retain(|_, value| !matches!(value, Json::Null));
    Ok(fields)
}

// add_openai_metadata runs after the overlay: a non-object attributes value passes through, an object keeps its first 16 string values in their order
fn clean_attributes(fields: &mut IndexMap<String, Json>) -> Result<(), FileTransportError> {
    let Some(Json::Object(attributes)) = fields.get("attributes") else {
        return Ok(());
    };
    let map = attributes
        .iter()
        .map(|(name, value)| serde_json::to_value(value).map(|value| (name.clone(), value)))
        .collect::<Result<IndexMap<String, _>, _>>()
        .map_err(FileTransportError::Encode)?;
    let Some(cleaned) = metadata_filter_limit(Some(&map)) else {
        return Ok(());
    };
    let kept = cleaned
        .into_iter()
        .map(|(name, text)| (name, Json::Text(text)))
        .collect();
    fields.insert("attributes".to_owned(), Json::Object(kept));
    Ok(())
}

// httpx writes every query value as text and a list as one pair per item
pub(crate) fn query_pairs(
    fields: &IndexMap<String, Json>,
) -> Result<Vec<(String, String)>, FileTransportError> {
    let mut pairs = Vec::with_capacity(fields.len());
    for (name, value) in fields {
        let items: &[Json] = match value {
            Json::List(items) => items.as_slice(),
            single => std::slice::from_ref(single),
        };
        for item in items {
            pairs.push((name.clone(), query_text(name, item)?));
        }
    }
    Ok(pairs)
}

fn query_text(name: &str, value: &Json) -> Result<String, FileTransportError> {
    match value {
        Json::Text(text) => Ok(text.clone()),
        Json::Number(number) => Ok(number.to_string()),
        Json::Bool(flag) => Ok(flag.to_string()),
        Json::Null | Json::List(_) | Json::Object(_) => Err(FileTransportError::QueryValue {
            name: name.to_owned(),
        }),
    }
}
