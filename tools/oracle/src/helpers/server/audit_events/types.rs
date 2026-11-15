use std::fmt::Display;

use betterlite_config::sections::settings::JsonValue;
use serde::Deserialize;

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "aspect", rename_all = "snake_case")]
pub(crate) enum Input {
    Policy(PolicyCall),
    Identity(IdentityCall),
    Record(RecordCall),
    Dispatch(DispatchCall),
    SinkDisabled(SinkCall),
}

#[derive(Debug, Deserialize)]
pub(crate) struct PolicyCall {
    pub(super) explicit: Option<bool>,
    pub(super) store: Option<bool>,
    pub(super) env: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct IdentityCall {
    pub(super) litellm_changed_by: Option<String>,
    pub(super) user_id: Option<String>,
    pub(super) metadata: Option<JsonValue>,
    pub(super) team_metadata: Option<JsonValue>,
    pub(super) admin_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RecordCall {
    pub(super) record: JsonValue,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DispatchCall {
    pub(super) record: JsonValue,
    pub(super) callbacks: Vec<CallbackCall>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum CallbackCall {
    Instance {
        label: String,
        #[serde(default)]
        fails: bool,
    },
    Named {
        name: String,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct SinkCall {
    pub(super) record: JsonValue,
    pub(super) object: ObjectCall,
    pub(super) caller: CallerCall,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ObjectCall {
    pub(super) object_id: String,
    pub(super) action: String,
    pub(super) table_name: String,
    pub(super) before_value: Option<String>,
    pub(super) after_value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CallerCall {
    pub(super) user_id: Option<String>,
    pub(super) api_key: Option<String>,
}

pub(super) fn failure<Message>(class: &str, message: Message) -> Box<HelperError>
where
    Message: Display,
{
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}
