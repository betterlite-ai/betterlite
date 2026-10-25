use betterlite_context::cloud_storage::{
    self,
    policy::{self, LegacyCloudSetting, TrustedCloudCredentials},
};
use serde_json::{Value, json};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let function = input
            .get("function")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = match function {
            "cloud_uri_detection" => {
                json!({"value": cloud_storage::is_managed_cloud_storage_uri(string(input, "value")?)})
            }
            "object_component_sanitization" => sanitize_component(input),
            "object_path_sanitization" => sanitize_path(input),
            "cloud_object_encoding" => {
                json!({"gcs": cloud_storage::encode_gcs_object_name_for_url(string(input, "value")?), "s3": cloud_storage::encode_s3_object_key_for_url(string(input, "value")?)})
            }
            "configured_bucket_split" => split_bucket(input)?,
            "object_path_validation" => validate_path(input)?,
            "legacy_trusted_policy" => {
                let credentials = trusted_credentials(input.get("params"));
                json!({"value": policy::allow_legacy(credentials.as_ref())})
            }
            "managed_id_authorization" | "legacy_prefix_boundary" => authorize(input)?,
            "encoded_traversal_order" => {
                match cloud_storage::decode_cloud_object_reference(string(input, "value")?) {
                    Ok((bucket, object)) => json!({"bucket": bucket, "object": object}),
                    Err(error) => return failure(error.to_string()),
                }
            }
            _ => return Self::not_built("cloud_storage"),
        };
        Ok(result)
    }
}

fn string<'input>(
    input: &'input Value,
    name: &str,
) -> Result<&'input str, Box<crate::helpers::HelperError>> {
    let required = name.to_owned();
    input.get(name).and_then(Value::as_str).ok_or_else(|| {
        Box::new(crate::helpers::HelperError {
            class: "InvalidInput".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: format!("{required} must be a string"),
        })
    })
}

fn sanitize_component(input: &Value) -> Value {
    let result = json!({"value": cloud_storage::sanitize_component(input.get("value").and_then(Value::as_str), input.get("fallback").and_then(Value::as_str).unwrap_or("file"))});
    result
}

fn sanitize_path(input: &Value) -> Value {
    let result = json!({"value": cloud_storage::sanitize_path(input.get("value").and_then(Value::as_str), input.get("fallback").and_then(Value::as_str).unwrap_or("file"))});
    result
}

fn split_bucket(input: &Value) -> Result<Value, Box<crate::helpers::HelperError>> {
    match cloud_storage::split_bucket_name(string(input, "value")?) {
        Ok((bucket, prefix)) => Ok(json!({"bucket": bucket, "prefix": prefix})),
        Err(error) => failure(error.to_string()),
    }
}

fn validate_path(input: &Value) -> Result<Value, Box<crate::helpers::HelperError>> {
    match cloud_storage::validate_cloud_object_path(string(input, "value")?) {
        Ok(()) => Ok(json!({"ok": true})),
        Err(error) => failure(error.to_string()),
    }
}

fn authorize(input: &Value) -> Result<Value, Box<crate::helpers::HelperError>> {
    let prefixes = input
        .get("prefixes")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    match cloud_storage::validate_managed_cloud_file_id(
        string(input, "file_id")?,
        string(input, "scheme")?,
        string(input, "bucket")?,
        &prefixes,
        input
            .get("legacy")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ) {
        Ok((bucket, object)) => Ok(json!({"bucket": bucket, "object": object})),
        Err(error) => failure(error.to_string()),
    }
}

fn failure(message: String) -> Result<Value, Box<crate::helpers::HelperError>> {
    let error_type = Some(Some("ValueError".to_owned()));
    Err(Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type,
        param: None,
        code: None,
        message,
    }))
}

fn trusted_credentials(value: Option<&Value>) -> Option<TrustedCloudCredentials> {
    let Value::Object(params) = value? else {
        return None;
    };
    let Value::Object(credentials) = params.get("_litellm_internal_model_credentials")? else {
        return None;
    };
    let setting = match credentials.get("allow_legacy_cloud_file_ids") {
        Some(Value::Bool(setting)) => LegacyCloudSetting::Boolean(*setting),
        Some(Value::String(setting)) => LegacyCloudSetting::Text(setting.clone()),
        _ => LegacyCloudSetting::Other,
    };
    Some(TrustedCloudCredentials {
        allow_legacy_cloud_file_ids: setting,
    })
}

mod tests;
