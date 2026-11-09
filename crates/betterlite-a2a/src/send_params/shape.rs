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
const PUSH_CONFIG_NAMES: &[&str] = &[
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
    PushConfig,
    Authentication,
}

pub(super) fn prepare(params: &AgentJson) -> Result<AgentJson, SendParamsError> {
    if !params.is_object() {
        return Err(SendParamsError::NotAnObject);
    }
    let mut prepared = params.clone();
    record(&mut prepared, Record::Params)?;
    Ok(prepared)
}

// serde's derived structs also read a JSON array by position, so every record position is checked
fn record(value: &mut AgentJson, kind: Record) -> Result<(), SendParamsError> {
    let Some(fields) = value.as_object_mut() else {
        return Err(SendParamsError::NotAnObjectAt {
            field: kind.field(),
        });
    };
    for (json, proto) in kind.aliases() {
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

// pydantic reads an int field from a JSON integer, an integral float or a numeric string
fn narrow_history_length(value: &mut AgentJson) {
    let number = match value {
        AgentJson::Number(number) => number.as_f64(),
        AgentJson::String(text) => text.parse::<f64>().ok(),
        AgentJson::Null | AgentJson::Bool(_) | AgentJson::Array(_) | AgentJson::Object(_) => return,
    };
    let Some(float) = number else {
        return;
    };
    if float.fract().to_bits() != 0.0_f64.to_bits() {
        return;
    }
    let Some(integer) = float.to_i64() else {
        return;
    };
    *value = AgentJson::from(integer);
}

fn children(value: &mut AgentJson, kind: Record) -> Result<(), SendParamsError> {
    match kind {
        Record::Params => {
            descend(value, &["message"], Record::Message)?;
            descend(value, &["configuration"], Record::Configuration)
        }
        Record::Message => descend_list(value, &["parts"], Record::Part),
        Record::Part => descend(value, &["file"], Record::File),
        Record::Configuration => descend(value, PUSH_CONFIG_NAMES, Record::PushConfig),
        Record::PushConfig => descend(value, &["authentication"], Record::Authentication),
        Record::File | Record::Authentication => Ok(()),
    }
}

fn descend(value: &mut AgentJson, names: &[&str], kind: Record) -> Result<(), SendParamsError> {
    let Some(fields) = value.as_object_mut() else {
        return Ok(());
    };
    let Some(name) = names
        .iter()
        .find(|name| fields.contains_key(**name))
        .copied()
    else {
        return Ok(());
    };
    let Some(child) = fields.get_mut(name) else {
        return Ok(());
    };
    if child.is_null() {
        return Ok(());
    }
    record(child, kind)
}

fn descend_list(
    value: &mut AgentJson,
    names: &[&str],
    kind: Record,
) -> Result<(), SendParamsError> {
    let Some(fields) = value.as_object_mut() else {
        return Ok(());
    };
    let Some(name) = names
        .iter()
        .find(|name| fields.contains_key(**name))
        .copied()
    else {
        return Ok(());
    };
    let Some(child) = fields.get_mut(name) else {
        return Ok(());
    };
    let AgentJson::Array(items) = child else {
        return Ok(());
    };
    for item in items {
        record(item, kind)?;
    }
    Ok(())
}

impl Record {
    const fn field(self) -> &'static str {
        match self {
            Self::Params => "params",
            Self::Message => "message",
            Self::Part => "message.parts",
            Self::File => "message.parts.file",
            Self::Configuration => "configuration",
            Self::PushConfig => "configuration.taskPushNotificationConfig",
            Self::Authentication => "configuration.taskPushNotificationConfig.authentication",
        }
    }

    const fn aliases(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Message => MESSAGE_ALIASES,
            Self::Part => PART_ALIASES,
            Self::File => FILE_ALIASES,
            Self::Configuration => CONFIG_ALIASES,
            Self::Params | Self::PushConfig | Self::Authentication => &[],
        }
    }
}
