use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "kind", content = "value")]
pub(crate) enum Input {
    #[serde(rename = "user")]
    User(serde_json::Value),
    #[serde(rename = "group")]
    Group(serde_json::Value),
    #[serde(rename = "patch")]
    Patch(serde_json::Value),
    #[serde(rename = "schema")]
    Schema(serde_json::Value),
    #[serde(rename = "resource_type")]
    ResourceType(serde_json::Value),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let make_error = |class: &str, message: String| {
            Box::new(crate::helpers::HelperError {
                class: class.to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message,
            })
        };
        if let Input::Schema(value) = input {
            let record: betterlite_server::scim_records::discovery::ScimSchema =
                serde_json::from_value(value.clone())
                    .map_err(|source| make_error("ValidationError", source.to_string()))?;
            return serde_json::to_value(record)
                .map_err(|source| make_error("SerializationError", source.to_string()));
        }
        if let Input::ResourceType(value) = input {
            let record: betterlite_server::scim_records::discovery::ScimResourceType =
                serde_json::from_value(value.clone())
                    .map_err(|source| make_error("ValidationError", source.to_string()))?;
            return serde_json::to_value(record)
                .map_err(|source| make_error("SerializationError", source.to_string()));
        }
        let Some((value, kind)) = (match input {
            Input::User(value) => Some((
                value,
                betterlite_server::scim_records::models::ResourceKind::User,
            )),
            Input::Group(value) => Some((
                value,
                betterlite_server::scim_records::models::ResourceKind::Group,
            )),
            Input::Patch(value) => Some((
                value,
                betterlite_server::scim_records::models::ResourceKind::Patch,
            )),
            Input::Schema(_) | Input::ResourceType(_) => None,
        }) else {
            return Err(make_error(
                "InvalidInput",
                "unknown SCIM resource kind".to_owned(),
            ));
        };
        let bytes = serde_json::to_vec(value)
            .map_err(|source| make_error("InvalidInput", source.to_string()))?;
        let decoded = betterlite_server::scim_records::decode_scim_resource(&bytes, kind)
            .map_err(|source| make_error("ValidationError", source.to_string()))?;
        serde_json::to_value(decoded)
            .map_err(|source| make_error("SerializationError", source.to_string()))
    }
}
