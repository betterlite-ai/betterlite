pub(crate) struct Op;

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    let class_name = class.to_owned();
    Box::new(crate::helpers::HelperError {
        class: class_name,
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let kind = input
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "kind must be a string".to_owned(),
            })?;
        let value = input
            .get("value")
            .cloned()
            .ok_or_else(|| crate::helpers::HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "value is required".to_owned(),
            })?;
        let resource_kind = match kind {
            "user" => betterlite_server::scim_records::models::ResourceKind::User,
            "group" => betterlite_server::scim_records::models::ResourceKind::Group,
            "patch" => betterlite_server::scim_records::models::ResourceKind::Patch,
            _ => return Self::not_built("unknown scim resource kind"),
        };
        let bytes = serde_json::to_vec(&value)
            .map_err(|source| helper_error("InvalidInput", source.to_string()))?;
        let decoded = betterlite_server::scim_records::decode_scim_resource(&bytes, resource_kind)
            .map_err(|source| helper_error("ValidationError", source.to_string()))?;
        serde_json::to_value(decoded)
            .map_err(|source| helper_error("SerializationError", source.to_string()))
    }
}
