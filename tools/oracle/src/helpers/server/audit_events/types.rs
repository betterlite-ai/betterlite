use std::fmt::Display;

use betterlite_config::sections::settings::JsonValue;
use serde::Deserialize;

use betterlite_oracle_helper_api::error::HelperError;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) aspect: Aspect,
    pub(super) explicit: Option<bool>,
    pub(super) store: Option<bool>,
    pub(super) env: Option<String>,
    pub(super) litellm_changed_by: Option<String>,
    pub(super) user_id: Option<String>,
    pub(super) metadata: Option<JsonValue>,
    pub(super) team_metadata: Option<JsonValue>,
    pub(super) admin_name: Option<String>,
    #[serde(default)]
    pub(super) record: JsonValue,
    #[serde(default)]
    pub(super) callbacks: Vec<CallbackCall>,
    #[serde(default)]
    pub(super) object_id: String,
    #[serde(default)]
    pub(super) action: String,
    #[serde(default)]
    pub(super) table_name: String,
    pub(super) before_value: Option<String>,
    pub(super) after_value: Option<String>,
    pub(super) api_key: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Aspect {
    Policy,
    Identity,
    Record,
    Dispatch,
    SinkDisabled,
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
