use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use indexmap::IndexMap;

mod tests;

#[derive(Debug)]
pub struct Event<'call> {
    pub kwargs: &'call Json,
    pub response: Option<&'call Json>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    #[error("the callback kwargs are not a JSON object")]
    Kwargs,
    #[error("the callback kwargs carry no {field}")]
    Missing { field: &'static str },
    #[error("the callback field {field} is not a JSON object")]
    NotObject { field: &'static str },
    #[error("the response object is not a JSON object")]
    Response,
}

pub const fn kwargs_object(kwargs: &Json) -> Result<&IndexMap<String, Json>, PayloadError> {
    if let Json::Object(fields) = kwargs {
        Ok(fields)
    } else {
        Err(PayloadError::Kwargs)
    }
}

pub fn object_field<'fields>(
    fields: &'fields IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<&'fields IndexMap<String, Json>>, PayloadError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(nested)) => Ok(Some(nested)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(PayloadError::NotObject { field: name })
        }
    }
}

pub fn metadata(
    fields: &IndexMap<String, Json>,
) -> Result<Option<&IndexMap<String, Json>>, PayloadError> {
    let Some(params) = object_field(fields, "litellm_params")? else {
        return Ok(None);
    };
    object_field(params, "metadata")
}

#[must_use]
pub fn response_dump(response: Option<&Json>) -> Json {
    match response {
        None | Some(Json::Null) => Json::Object(IndexMap::new()),
        Some(dump) => dump.clone(),
    }
}

#[must_use]
pub fn usage_token(response: &Json, name: &'static str) -> Json {
    let Json::Object(fields) = response else {
        return Json::Null;
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        return Json::Null;
    };
    usage.get(name).cloned().unwrap_or(Json::Null)
}
