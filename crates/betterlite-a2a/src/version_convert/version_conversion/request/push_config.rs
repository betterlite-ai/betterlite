use betterlite_core::agents::json::AgentJson;

use crate::version_convert::{JsonObject, to_object};

use crate::version_convert::version_conversion::error::ConvertError;
use crate::version_convert::version_conversion::fields::{Copy, copy, insert, object, string};

pub(super) fn lower_set(params: &JsonObject) -> Result<JsonObject, ConvertError> {
    let Some(flat) = flatten(params)? else {
        return Ok(params.clone());
    };
    let mut config = JsonObject::new();
    copy(Copy::TextOrDefault, &mut config, "url", &flat, "url")?;
    copy(Copy::NonEmptyText, &mut config, "id", &flat, "id")?;
    copy(Copy::NonEmptyText, &mut config, "token", &flat, "token")?;
    insert(&mut config, "authentication", authentication(&flat)?);
    let mut out = JsonObject::new();
    copy(Copy::TextOrDefault, &mut out, "taskId", &flat, "taskId")?;
    out.insert(
        "pushNotificationConfig".to_owned(),
        AgentJson::Object(config.into_iter().collect()),
    );
    Ok(out)
}

pub(super) fn lower_get(params: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut out = JsonObject::new();
    copy_config_ids(&mut out, params, Copy::NonEmptyText)?;
    Ok(out)
}

pub(super) fn lower_list(params: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut out = JsonObject::new();
    copy(Copy::TextOrDefault, &mut out, "id", params, "taskId")?;
    Ok(out)
}

pub(super) fn lower_delete(params: &JsonObject) -> Result<JsonObject, ConvertError> {
    let mut out = JsonObject::new();
    copy_config_ids(&mut out, params, Copy::TextOrDefault)?;
    Ok(out)
}

fn copy_config_ids(
    out: &mut JsonObject,
    params: &JsonObject,
    mode: Copy,
) -> Result<(), ConvertError> {
    copy(Copy::TextOrDefault, out, "id", params, "taskId")?;
    copy(mode, out, "pushNotificationConfigId", params, "id")?;
    Ok(())
}

fn authentication(fields: &JsonObject) -> Result<Option<AgentJson>, ConvertError> {
    let Some(auth) = object(fields, "authentication")? else {
        return Ok(None);
    };
    let mut schemes = Vec::new();
    if let Some(scheme) = string(&auth, "scheme")?.filter(|text| !text.is_empty()) {
        schemes.push(AgentJson::String(scheme));
    }
    let mut out = JsonObject::new();
    out.insert("schemes".to_owned(), AgentJson::Array(schemes));
    copy(
        Copy::NonEmptyText,
        &mut out,
        "credentials",
        &auth,
        "credentials",
    )?;
    Ok(Some(AgentJson::Object(out.into_iter().collect())))
}

fn flatten(params: &JsonObject) -> Result<Option<JsonObject>, ConvertError> {
    let mut flat = params.clone();
    let config = flat.remove("config").filter(|value| !value.is_null());
    let push_config = flat
        .remove("pushNotificationConfig")
        .filter(|value| !value.is_null());
    let Some(nested) = config.or(push_config) else {
        return Ok(None);
    };
    let Some(mut nested_object) = to_object(&nested) else {
        return Ok(None);
    };
    if let Some(AgentJson::String(parent)) = flat.remove("parent")
        && !flat.contains_key("taskId")
        && let Some(rest) = parent.strip_prefix("tasks/")
    {
        let task_id = rest.split('/').next().unwrap_or_default();
        flat.insert("taskId".to_owned(), AgentJson::String(task_id.to_owned()));
    }
    match flat.remove("configId") {
        None | Some(AgentJson::Null) => {}
        Some(AgentJson::String(config_id)) if config_id.is_empty() => {}
        Some(AgentJson::String(config_id)) => {
            if !nested_object.contains_key("id") {
                nested_object.insert("id".to_owned(), AgentJson::String(config_id));
            }
        }
        Some(_) => {
            return Err(ConvertError::Field {
                field: "configId".to_owned(),
                expected: "a string",
            });
        }
    }
    for (key, value) in nested_object {
        flat.insert(key, value);
    }
    Ok(Some(flat))
}
