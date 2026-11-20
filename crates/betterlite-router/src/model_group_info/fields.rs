use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::model_group_info::cost::decimal_from_lexeme;
use crate::model_group_info::error::GroupInfoError;
use crate::model_group_info::group::ClientsideAuth;

const AUTH: &str = "configurable_clientside_auth_params";

// A float-typed ModelGroupInfo field goes out as the double Python's model_dump() writes.
#[must_use]
pub fn json_float(value: Option<&Decimal>) -> Json {
    value
        .and_then(Decimal::to_f64)
        .and_then(Number::from_f64)
        .map_or(Json::Null, Json::Number)
}

pub(crate) fn read_field<T>(
    map: &JsonObject,
    name: &'static str,
    parse: fn(&Json, &'static str) -> Result<Option<T>, GroupInfoError>,
) -> Result<Option<T>, GroupInfoError> {
    let Some(value) = map.get(name) else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    parse(value, name)
}

pub(crate) fn parse_decimal(
    value: &Json,
    name: &'static str,
) -> Result<Option<Decimal>, GroupInfoError> {
    let Json::Number(number) = value else {
        return Err(GroupInfoError::Field { field: name });
    };
    Ok(Some(decimal_from_lexeme(&number.to_string())?))
}

pub(crate) fn parse_i64(value: &Json, name: &'static str) -> Result<Option<i64>, GroupInfoError> {
    let Json::Number(number) = value else {
        return Err(GroupInfoError::Field { field: name });
    };
    if let Some(found) = number.as_i64() {
        return Ok(Some(found));
    }
    let unsigned = number
        .as_u64()
        .ok_or(GroupInfoError::Field { field: name })?;
    i64::try_from(unsigned)
        .map(Some)
        .map_err(|_error| GroupInfoError::Field { field: name })
}

pub fn mode_value(map: &JsonObject) -> Result<Option<String>, GroupInfoError> {
    match map.get("mode") {
        None => Ok(Some("chat".to_owned())),
        Some(Json::Null) => Ok(None),
        Some(Json::Text(mode)) => Ok(Some(mode.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(GroupInfoError::Field { field: "mode" })
        }
    }
}

pub fn text_list(value: &Json, field: &'static str) -> Result<Vec<String>, GroupInfoError> {
    let Json::List(items) = value else {
        return Err(GroupInfoError::Field { field });
    };
    let mut names = Vec::with_capacity(items.len());
    for item in items {
        let Json::Text(name) = item else {
            return Err(GroupInfoError::Field { field });
        };
        names.push(name.clone());
    }
    Ok(names)
}

pub fn constructor_params(map: &JsonObject) -> Result<Option<Vec<String>>, GroupInfoError> {
    match map.get("supported_openai_params") {
        None => Ok(Some(Vec::new())),
        Some(Json::Null) => Ok(None),
        Some(value) => Ok(Some(text_list(value, "supported_openai_params")?)),
    }
}

pub fn replacement_params(map: &JsonObject) -> Result<Option<Vec<String>>, GroupInfoError> {
    match map.get("supported_openai_params") {
        None | Some(Json::Null) => Ok(None),
        Some(value) => Ok(Some(text_list(value, "supported_openai_params")?)),
    }
}

// The constructor path validates an api_base item and keeps only that key; the params path keeps the raw object.
pub fn auth_field(
    map: &JsonObject,
    keep_extras: bool,
) -> Result<Option<Vec<ClientsideAuth>>, GroupInfoError> {
    match map.get(AUTH) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => Ok(Some(auth_list(value, keep_extras)?)),
    }
}

fn auth_list(value: &Json, keep_extras: bool) -> Result<Vec<ClientsideAuth>, GroupInfoError> {
    let Json::List(items) = value else {
        return Err(GroupInfoError::Field { field: AUTH });
    };
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Json::Text(name) => entries.push(ClientsideAuth::Name(name.clone())),
            Json::Object(fields) => {
                entries.push(ClientsideAuth::Item(item_object(fields, keep_extras)?));
            }
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
                return Err(GroupInfoError::Field { field: AUTH });
            }
        }
    }
    Ok(entries)
}

fn item_object(fields: &JsonObject, keep_extras: bool) -> Result<JsonObject, GroupInfoError> {
    let Some(Json::Text(api_base)) = fields.get("api_base") else {
        return Err(GroupInfoError::Field { field: AUTH });
    };
    if keep_extras {
        return Ok(fields.clone());
    }
    let mut only_api_base = JsonObject::new();
    only_api_base.insert("api_base".to_owned(), Json::Text(api_base.clone()));
    Ok(only_api_base)
}
