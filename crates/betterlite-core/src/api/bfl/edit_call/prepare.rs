use std::mem;

use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageParams;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};

use super::EditCall;
use super::error::EditCallError;
use crate::api::bfl::edit::media::{MaskInput, SuppliedMedia};
use crate::api::bfl::edit::{complete_url, transform_image_edit_request};
use crate::api::bfl::generation::validate_environment;

pub struct PreparedEdit {
    pub url: String,
    pub headers: HeaderMap,
    pub body: ImageParams,
}

pub fn prepare(call: &mut EditCall<'_>) -> Result<PreparedEdit, EditCallError> {
    let mut seeded = HeaderMap::new();
    for (name, value) in optional_extra_headers(call.optional_params)? {
        insert_header(&mut seeded, &name, &value)?;
    }
    // the credential and the two JSON headers overwrite whatever the optional params seeded
    let headers = validate_environment(seeded, call.params.api_key.as_ref(), call.configured_key)?;
    let url = complete_url(
        call.params.api_base.as_deref(),
        call.model,
        call.configured_base,
    )?;
    // an empty outer list is the handler's own 400, raised before the transform can index it
    if let Some(SuppliedMedia::List(items)) = call.image.as_deref()
        && items.is_empty()
    {
        return Err(EditCallError::NoImageProvided);
    }
    let image = call.image.as_deref_mut();
    let mask = mem::replace(&mut call.mask, MaskInput::Absent);
    // the BFL handler's async entry point runs the same transform as its sync one
    let prompt = Some(call.prompt.unwrap_or(""));
    let form =
        transform_image_edit_request(prompt, image, mask, call.optional_params, call.max_depth)?;
    Ok(PreparedEdit {
        url,
        headers,
        body: form.data,
    })
}

// LiteLLM hands image_edit_optional_request_params.get("extra_headers", {}) or {} to validate_environment as the starting dict
fn optional_extra_headers(params: &ImageParams) -> Result<Vec<(String, String)>, EditCallError> {
    let fields = match params.get("extra_headers") {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::Object(fields)) => fields,
        Some(_) => return Err(EditCallError::InnerHeadersType),
    };
    let mut headers = Vec::with_capacity(fields.len());
    for (name, value) in fields {
        let Json::Text(text) = value else {
            return Err(EditCallError::InnerHeaderValue { name: name.clone() });
        };
        headers.push((name.clone(), text.clone()));
    }
    Ok(headers)
}

pub(super) fn insert_header(
    headers: &mut HeaderMap,
    name: &str,
    value: &str,
) -> Result<(), EditCallError> {
    let header_name = HeaderName::try_from(name).map_err(|source| EditCallError::HeaderName {
        name: name.to_owned(),
        source,
    })?;
    // httpx encodes header values as ASCII, which a HeaderValue would not enforce
    if !value.is_ascii() {
        return Err(EditCallError::HeaderNotAscii {
            name: header_name.as_str().to_owned(),
        });
    }
    let header_value =
        HeaderValue::try_from(value).map_err(|source| EditCallError::HeaderValue {
            name: header_name.as_str().to_owned(),
            source,
        })?;
    headers.insert(header_name, header_value);
    Ok(())
}
