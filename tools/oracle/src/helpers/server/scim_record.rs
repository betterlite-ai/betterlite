use betterlite_server::scim_records::discovery::{ResourceType, Schema};
use betterlite_server::scim_records::user::Active;
use betterlite_server::scim_records::{ResourceKind, ScimResource, decode_scim_resource};
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[expect(
    clippy::disallowed_types,
    reason = "The SCIM oracle transports arbitrary model input as JSON."
)]
type ScimJson = serde_json::Value;

#[derive(Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum Input {
    User(ScimJson),
    Group(ScimJson),
    Patch(ScimJson),
    Schema(ScimJson),
    ResourceType(ScimJson),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let dumped = match input {
            Input::User(value) => decode(value, ResourceKind::User),
            Input::Group(value) => decode(value, ResourceKind::Group),
            Input::Patch(value) => decode(value, ResourceKind::Patch),
            Input::Schema(value) => Schema::deserialize(value).and_then(serde_json::to_value),
            Input::ResourceType(value) => {
                ResourceType::deserialize(value).and_then(serde_json::to_value)
            }
        };
        dumped.map_err(|error| {
            Box::new(HelperError {
                class: "ValidationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn decode(value: &ScimJson, kind: ResourceKind) -> Result<ScimJson, serde_json::Error> {
    let bytes = serde_json::to_vec(value)?;
    match decode_scim_resource(&bytes, kind) {
        Ok(ScimResource::User(user)) => Ok(json!({
            "record": serde_json::to_value(&user)?,
            "active_set": matches!(user.active, Active::Sent(_)),
        })),
        Ok(resource) => serde_json::to_value(resource),
        Err(error) => Err(serde::de::Error::custom(error)),
    }
}
