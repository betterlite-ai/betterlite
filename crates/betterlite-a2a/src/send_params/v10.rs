mod convert;
mod values;

use betterlite_core::agents::json::AgentJson;
use serde::Deserialize;
use serde::de::IntoDeserializer as _;
use serde_with::{DefaultOnNull, serde_as};

use crate::send_params::error::SendParamsError;
use crate::send_params::v03::{JsonMap, MessageSendParamsV03};

// protobuf JSON names a field in camelCase or its proto name, skips undeclared keys and reads null as absent
#[serde_as]
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct RequestV10 {
    message: Option<MessageV10>,
    configuration: Option<ConfigV10>,
    metadata: Option<JsonMap>,
}

#[serde_as]
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MessageV10 {
    #[serde_as(as = "DefaultOnNull")]
    #[serde(alias = "message_id")]
    message_id: String,
    #[serde_as(as = "DefaultOnNull")]
    #[serde(alias = "context_id")]
    context_id: String,
    #[serde_as(as = "DefaultOnNull")]
    #[serde(alias = "task_id")]
    task_id: String,
    role: Option<RoleV10>,
    #[serde_as(as = "DefaultOnNull")]
    parts: Vec<PartV10>,
    metadata: Option<JsonMap>,
    #[serde_as(as = "DefaultOnNull")]
    extensions: Vec<String>,
    #[serde_as(as = "DefaultOnNull")]
    #[serde(alias = "reference_task_ids")]
    reference_task_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RoleV10 {
    Number(i64),
    Name(String),
}

#[serde_as]
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PartV10 {
    text: Option<String>,
    raw: Option<String>,
    url: Option<String>,
    data: DataV10,
    metadata: Option<JsonMap>,
    #[serde_as(as = "DefaultOnNull")]
    #[serde(alias = "media_type")]
    media_type: String,
    #[serde_as(as = "DefaultOnNull")]
    filename: String,
}

// an explicit null data value is a Value holding null, so the value variant is tried before absence
#[derive(Default, Deserialize)]
#[serde(untagged)]
enum DataV10 {
    Value(AgentJson),
    #[default]
    Absent,
}

#[serde_as]
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ConfigV10 {
    #[serde_as(as = "DefaultOnNull")]
    #[serde(alias = "accepted_output_modes")]
    accepted_output_modes: Vec<String>,
    #[serde(alias = "task_push_notification_config")]
    task_push_notification_config: Option<PushConfigV10>,
    #[serde(alias = "history_length")]
    history_length: Option<IntegerV10>,
    #[serde_as(as = "DefaultOnNull")]
    #[serde(alias = "return_immediately")]
    return_immediately: bool,
}

// protobuf JSON reads an int32 from a quoted decimal as well as from a number
#[derive(Deserialize)]
#[serde(untagged)]
enum IntegerV10 {
    Number(serde_json::Number),
    Text(String),
}

#[serde_as]
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PushConfigV10 {
    #[serde_as(as = "DefaultOnNull")]
    url: String,
    #[serde_as(as = "DefaultOnNull")]
    id: String,
    #[serde_as(as = "DefaultOnNull")]
    token: String,
    authentication: Option<AuthV10>,
}

#[serde_as]
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct AuthV10 {
    #[serde_as(as = "DefaultOnNull")]
    scheme: String,
    #[serde_as(as = "DefaultOnNull")]
    credentials: String,
}

pub(crate) fn parse_and_convert(
    params: &AgentJson,
) -> Result<MessageSendParamsV03, SendParamsError> {
    let request = serde_path_to_error::deserialize::<_, RequestV10>(params.into_deserializer())
        .map_err(|error| {
            let path = error.path().to_string();
            SendParamsError::Malformed {
                path,
                source: error.into_inner(),
            }
        })?;
    convert::request(request)
}
