use betterlite_server::scim_records::discovery::{ResourceType, Schema};
use betterlite_server::scim_records::user::{Active, User};
use betterlite_server::scim_records::{ResourceKind, ScimResource, decode_scim_resource};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use crate::helpers::{Helper, HelperError, HelperResult};
pub(crate) struct Op;
#[derive(Deserialize)]
pub(crate) struct Input {
    kind: Kind,
    value: Box<RawValue>,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    User,
    Group,
    Patch,
    Schema,
    ResourceType,
}
#[derive(Serialize)]
#[serde(untagged)]
enum Output {
    User { record: Box<User>, active_set: bool },
    Resource(ScimResource),
}
impl Helper for Op {
    type Input = Input;
    fn run(input: &Input) -> HelperResult {
        let value = input.value.get();
        let dumped = match input.kind {
            Kind::User => decode(value, ResourceKind::User).and_then(serde_json::to_value),
            Kind::Group => decode(value, ResourceKind::Group).and_then(serde_json::to_value),
            Kind::Patch => decode(value, ResourceKind::Patch).and_then(serde_json::to_value),
            Kind::Schema => serde_json::from_str::<Schema>(value).and_then(serde_json::to_value),
            Kind::ResourceType => {
                serde_json::from_str::<ResourceType>(value).and_then(serde_json::to_value)
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

fn decode(value: &str, kind: ResourceKind) -> Result<Output, serde_json::Error> {
    match decode_scim_resource(value.as_bytes(), kind) {
        Ok(ScimResource::User(user)) => Ok(Output::User {
            active_set: matches!(user.active, Active::Sent(_)),
            record: user,
        }),
        Ok(resource) => Ok(Output::Resource(resource)),
        Err(error) => Err(serde::de::Error::custom(error)),
    }
}
