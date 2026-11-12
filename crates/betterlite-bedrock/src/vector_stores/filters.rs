use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::vector_stores::error::VectorStoreError;

#[must_use]
pub fn map_operator_to_aws(operator: &str) -> &str {
    match operator {
        "eq" => "equals",
        "ne" => "notEquals",
        "gt" => "greaterThan",
        "gte" => "greaterThanOrEquals",
        "lt" => "lessThan",
        "lte" => "lessThanOrEquals",
        "in" => "in",
        "nin" => "notIn",
        other => other,
    }
}

pub fn map_operator_filter(filter: &Json) -> Result<Json, VectorStoreError> {
    let Json::Object(fields) = filter else {
        return Err(VectorStoreError::FilterNotObject);
    };
    let operator = match fields.get("operator") {
        Some(Json::Text(operator)) => operator,
        Some(_) => return Err(VectorStoreError::FilterOperatorNotString),
        None => return Err(VectorStoreError::FilterMissingField { field: "operator" }),
    };
    let key = fields
        .get("key")
        .ok_or(VectorStoreError::FilterMissingField { field: "key" })?;
    let value = fields
        .get("value")
        .ok_or(VectorStoreError::FilterMissingField { field: "value" })?;
    let mut selected = IndexMap::new();
    selected.insert("key".to_owned(), key.clone());
    selected.insert("value".to_owned(), value.clone());
    let mut mapped = IndexMap::new();
    mapped.insert(
        map_operator_to_aws(operator).to_owned(),
        Json::Object(selected),
    );
    Ok(Json::Object(mapped))
}

pub fn map_and_or_filters(value: &Json) -> Result<Json, VectorStoreError> {
    let Json::Object(fields) = value else {
        return Err(VectorStoreError::FilterNotObject);
    };
    let mut mapped = IndexMap::new();
    if let Some(single) = map_compound(fields, "and", "andAll", &mut mapped)? {
        return Ok(single);
    }
    if let Some(single) = map_compound(fields, "or", "orAll", &mut mapped)? {
        return Ok(single);
    }
    Ok(Json::Object(mapped))
}

fn map_compound(
    fields: &IndexMap<String, Json>,
    key: &'static str,
    all: &'static str,
    mapped: &mut IndexMap<String, Json>,
) -> Result<Option<Json>, VectorStoreError> {
    let Some(value) = fields.get(key) else {
        return Ok(None);
    };
    let Json::List(children) = value else {
        return Err(VectorStoreError::FilterListNotArray { field: key });
    };
    if let [single] = children.as_slice() {
        return map_operator_filter(single).map(Some);
    }
    let mapped_children = children
        .iter()
        .map(map_operator_filter)
        .collect::<Result<Vec<_>, _>>()?;
    mapped.insert(all.to_owned(), Json::List(mapped_children));
    Ok(None)
}

pub fn map_openai_params(
    non_default_params: &IndexMap<String, Json>,
    optional_params: &mut IndexMap<String, Json>,
) -> Result<(), VectorStoreError> {
    for (param, value) in non_default_params {
        match param.as_str() {
            "max_num_results" => {
                optional_params.insert("numberOfResults".to_owned(), value.clone());
            }
            "filters" if !matches!(value, Json::Null) => {
                optional_params.insert("filters".to_owned(), mapped_filters(value)?);
            }
            _ => {}
        }
    }
    Ok(())
}

fn mapped_filters(value: &Json) -> Result<Json, VectorStoreError> {
    let Json::Object(fields) = value else {
        return Ok(Json::Null);
    };
    if fields.contains_key("operator") {
        return map_operator_filter(value);
    }
    if fields.contains_key("and") || fields.contains_key("or") {
        return map_and_or_filters(value);
    }
    Ok(value.clone())
}
