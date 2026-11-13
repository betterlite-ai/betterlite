use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::DEFAULT_APPLICATION_ID;
use super::error::NomaError;

pub(super) const fn as_object(value: &GuardrailValue) -> Option<&BTreeMap<String, GuardrailValue>> {
    if let GuardrailValue::Object(fields) = value {
        Some(fields)
    } else {
        None
    }
}

pub(super) fn as_object_mut(
    value: &mut GuardrailValue,
) -> Option<&mut BTreeMap<String, GuardrailValue>> {
    if let GuardrailValue::Object(fields) = value {
        Some(fields)
    } else {
        None
    }
}

pub(super) fn call_id(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<&str>, NomaError> {
    match request_data.get("litellm_call_id") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(id)) => Ok(Some(id)),
        Some(_) => Err(NomaError::Field {
            field: "litellm_call_id",
        }),
    }
}

pub(super) fn auth_field<'data>(
    user_auth: &'data BTreeMap<String, GuardrailValue>,
    name: &'static str,
) -> Result<Option<&'data str>, NomaError> {
    match user_auth.get(name) {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(GuardrailValue::Text(value)) => Ok(Some(value)),
        Some(_) => Err(NomaError::Field { field: name }),
    }
}

fn metadata(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<&BTreeMap<String, GuardrailValue>>, NomaError> {
    match request_data.get("metadata") {
        None | Some(GuardrailValue::Null) => Ok(None),
        Some(value) => as_object(value)
            .map(Some)
            .ok_or(NomaError::Field { field: "metadata" }),
    }
}

pub(super) fn application_header(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<&GuardrailValue>, NomaError> {
    let Some(metadata) = metadata(request_data)? else {
        return Ok(None);
    };
    let Some(headers_value) = metadata.get("headers") else {
        return Ok(None);
    };
    let Some(headers) = as_object(headers_value) else {
        return Err(NomaError::Field {
            field: "metadata.headers",
        });
    };
    Ok(headers
        .get("x-noma-application-id")
        .filter(|value| !matches!(value, GuardrailValue::Null)))
}

pub(super) fn requester_ip(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<&GuardrailValue>, NomaError> {
    let Some(metadata) = metadata(request_data)? else {
        return Ok(None);
    };
    Ok(metadata
        .get("requester_ip_address")
        .filter(|value| !matches!(value, GuardrailValue::Null)))
}

pub(super) fn application_id<'data>(
    request_data: &'data BTreeMap<String, GuardrailValue>,
    user_auth: &'data BTreeMap<String, GuardrailValue>,
    extra_data: &'data BTreeMap<String, GuardrailValue>,
    configured: Option<&'data str>,
) -> Result<GuardrailValue, NomaError> {
    if let Some(value) = extra_data
        .get("application_id")
        .filter(|value| !matches!(value, GuardrailValue::Null))
    {
        return Ok(value.clone());
    }
    if let Some(value) = application_header(request_data)? {
        return Ok(value.clone());
    }
    if let Some(configured) = configured {
        return Ok(GuardrailValue::Text(configured.to_owned()));
    }
    if let Some(alias) = auth_field(user_auth, "key_alias")? {
        return Ok(GuardrailValue::Text(alias.to_owned()));
    }
    Ok(GuardrailValue::Text(DEFAULT_APPLICATION_ID.to_owned()))
}
