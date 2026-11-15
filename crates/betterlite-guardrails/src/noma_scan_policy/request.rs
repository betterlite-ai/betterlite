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

pub(super) const fn as_object_mut(
    value: &mut GuardrailValue,
) -> Option<&mut BTreeMap<String, GuardrailValue>> {
    if let GuardrailValue::Object(fields) = value {
        Some(fields)
    } else {
        None
    }
}

pub(super) fn text<'data>(
    data: &'data BTreeMap<String, GuardrailValue>,
    name: &'static str,
) -> Result<Option<&'data str>, NomaError> {
    let Some(value) = data.get(name) else {
        return Ok(None);
    };
    match value {
        GuardrailValue::Text(found) => Ok(Some(found)),
        GuardrailValue::Null => Ok(None),
        GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => Err(NomaError::Field { field: name }),
    }
}

pub(super) fn metadata(
    request_data: &BTreeMap<String, GuardrailValue>,
) -> Result<Option<&BTreeMap<String, GuardrailValue>>, NomaError> {
    let Some(value) = request_data
        .get("metadata")
        .filter(|candidate| !matches!(candidate, GuardrailValue::Null))
    else {
        return Ok(None);
    };
    as_object(value)
        .map(Some)
        .ok_or(NomaError::Field { field: "metadata" })
}

pub(super) fn application_id<'data>(
    request_data: &'data BTreeMap<String, GuardrailValue>,
    user_auth: &'data BTreeMap<String, GuardrailValue>,
    extra_data: &'data BTreeMap<String, GuardrailValue>,
    configured: Option<&'data str>,
) -> Result<GuardrailValue, NomaError> {
    if let Some(value) = extra_data
        .get("application_id")
        .filter(|candidate| !matches!(candidate, GuardrailValue::Null))
    {
        return Ok(value.clone());
    }
    if let Some(metadata) = metadata(request_data)? {
        match metadata.get("headers") {
            None | Some(GuardrailValue::Null) => {}
            Some(value) => {
                let headers = as_object(value).ok_or(NomaError::Field {
                    field: "metadata.headers",
                })?;
                if let Some(header) = headers
                    .get("x-noma-application-id")
                    .filter(|candidate| !matches!(candidate, GuardrailValue::Null))
                {
                    return Ok(header.clone());
                }
            }
        }
    }
    if let Some(application) = configured {
        return Ok(GuardrailValue::Text(application.to_owned()));
    }
    if let Some(alias) = text(user_auth, "key_alias")? {
        return Ok(GuardrailValue::Text(alias.to_owned()));
    }
    Ok(GuardrailValue::Text(DEFAULT_APPLICATION_ID.to_owned()))
}
