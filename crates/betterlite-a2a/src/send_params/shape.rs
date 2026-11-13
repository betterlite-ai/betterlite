use betterlite_core::agents::json::AgentJson;
use num_traits::ToPrimitive as _;

use crate::send_params::error::SendParamsError;

const MESSAGE_ALIASES: &[(&str, &str)] = &[
    ("messageId", "message_id"),
    ("contextId", "context_id"),
    ("taskId", "task_id"),
    ("referenceTaskIds", "reference_task_ids"),
];
const PART_ALIASES: &[(&str, &str)] = &[("mediaType", "media_type")];
const FILE_ALIASES: &[(&str, &str)] = &[("mimeType", "mime_type")];
const CONFIG_ALIASES: &[(&str, &str)] = &[
    ("acceptedOutputModes", "accepted_output_modes"),
    ("historyLength", "history_length"),
    ("returnImmediately", "return_immediately"),
    (
        "taskPushNotificationConfig",
        "task_push_notification_config",
    ),
    ("pushNotificationConfig", "push_notification_config"),
];
const PUSH_CONFIG_KEYS: &[&str] = &[
    "taskPushNotificationConfig",
    "task_push_notification_config",
    "pushNotificationConfig",
    "push_notification_config",
];
#[derive(Clone, Copy)]
enum Record {
    Params,
    Message,
    Part,
    File,
    Configuration,
}

struct Spec {
    field: &'static str,
    aliases: &'static [(&'static str, &'static str)],
}

pub(super) fn prepare(params: &AgentJson) -> Result<AgentJson, SendParamsError> {
    if !params.is_object() {
        return Err(SendParamsError::NotAnObject);
    }
    let mut prepared = params.clone();
    record(&mut prepared, Record::Params)?;
    Ok(prepared)
}

// only params, message, each part and configuration are records on both paths, so only they are fixed up
fn record(value: &mut AgentJson, kind: Record) -> Result<(), SendParamsError> {
    let spec = kind.spec();
    let Some(fields) = value.as_object_mut() else {
        return Err(SendParamsError::NotAnObjectAt { field: spec.field });
    };
    for (json, proto) in spec.aliases {
        if fields.contains_key(*json) {
            fields.remove(*proto);
        }
    }
    if matches!(kind, Record::Configuration) {
        for name in ["historyLength", "history_length"] {
            if let Some(length) = fields.get_mut(name) {
                narrow_history_length(length);
            }
        }
    }
    children(value, kind)
}

// pydantic reads an int field from an integral float or a numeric string, and an integer stays exact
fn narrow_history_length(value: &mut AgentJson) {
    let integer = match value {
        AgentJson::Number(number) if number.is_f64() => number.as_f64().and_then(integral),
        AgentJson::String(text) => text
            .parse::<i64>()
            .ok()
            .or_else(|| text.parse::<f64>().ok().and_then(integral)),
        AgentJson::Number(_)
        | AgentJson::Null
        | AgentJson::Bool(_)
        | AgentJson::Array(_)
        | AgentJson::Object(_) => None,
    };
    if let Some(exact) = integer {
        *value = AgentJson::from(exact);
    }
}

fn integral(value: f64) -> Option<i64> {
    if value.fract().to_bits() != 0.0_f64.to_bits() {
        return None;
    }
    value.to_i64()
}

fn children(value: &mut AgentJson, kind: Record) -> Result<(), SendParamsError> {
    match kind {
        Record::Params => {
            descend(value, &["message"], Record::Message)?;
            descend(value, &["configuration"], Record::Configuration)
        }
        Record::Message => {
            let Some(parts) = child_mut(value, &["parts"]) else {
                return Ok(());
            };
            let AgentJson::Array(items) = parts else {
                return Ok(());
            };
            for item in items {
                record(item, Record::Part)?;
            }
            Ok(())
        }
        Record::Part => {
            // a non-object file only makes the 0.3 File variant fail, as the pydantic union does
            if let Some(file) = child_mut(value, &["file"]) {
                reject_array(file);
                if file.is_object() {
                    record(file, Record::File)?;
                }
            }
            Ok(())
        }
        Record::Configuration => {
            normalize_push_config(value);
            Ok(())
        }
        Record::File => Ok(()),
    }
}

// each push config key belongs to one path, so an array there must fail only the path that reads it
fn normalize_push_config(value: &mut AgentJson) {
    let Some(fields) = value.as_object_mut() else {
        return;
    };
    for name in PUSH_CONFIG_KEYS {
        let Some(push) = fields.get_mut(*name) else {
            continue;
        };
        reject_array(push);
        let AgentJson::Object(push_fields) = push else {
            continue;
        };
        if let Some(authentication) = push_fields.get_mut("authentication") {
            reject_array(authentication);
        }
    }
}

// a JSON array is not a record, and serde's derived structs would read it by position
fn reject_array(value: &mut AgentJson) {
    if value.is_array() {
        *value = AgentJson::Bool(false);
    }
}

fn descend(value: &mut AgentJson, names: &[&str], kind: Record) -> Result<(), SendParamsError> {
    let Some(child) = child_mut(value, names) else {
        return Ok(());
    };
    record(child, kind)
}

fn child_mut<'value>(
    value: &'value mut AgentJson,
    names: &[&str],
) -> Option<&'value mut AgentJson> {
    let fields = value.as_object_mut()?;
    let name = names
        .iter()
        .find(|name| fields.contains_key(**name))
        .copied()?;
    fields.get_mut(name).filter(|child| !child.is_null())
}

impl Record {
    const fn spec(self) -> Spec {
        match self {
            Self::Params => Spec {
                field: "params",
                aliases: &[],
            },
            Self::Message => Spec {
                field: "message",
                aliases: MESSAGE_ALIASES,
            },
            Self::Part => Spec {
                field: "message.parts",
                aliases: PART_ALIASES,
            },
            Self::File => Spec {
                field: "message.parts.file",
                aliases: FILE_ALIASES,
            },
            Self::Configuration => Spec {
                field: "configuration",
                aliases: CONFIG_ALIASES,
            },
        }
    }
}
