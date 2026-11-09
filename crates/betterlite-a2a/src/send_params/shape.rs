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

// serde's derived structs also read a JSON array by position, so every record position is checked
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
        Record::Part => descend(value, &["file"], Record::File),
        Record::Configuration => descend(value, PUSH_CONFIG_NAMES, Record::PushConfig),
        Record::PushConfig => descend(value, &["authentication"], Record::Authentication),
        Record::File | Record::Authentication => Ok(()),
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
            Self::PushConfig => Spec {
                field: "configuration.taskPushNotificationConfig",
                aliases: &[],
            },
            Self::Authentication => Spec {
                field: "configuration.taskPushNotificationConfig.authentication",
                aliases: &[],
            },
        }
    }
}
