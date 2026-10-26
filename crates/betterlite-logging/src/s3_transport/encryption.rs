use crate::s3_transport::option_value::OptionValue;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSideEncryption {
    pub algorithm: String,
    pub kms_key_id: Option<String>,
}

#[must_use]
pub fn resolve_server_side_encryption(
    algorithm: Option<&OptionValue>,
    kms_key_id: Option<&OptionValue>,
) -> Option<ServerSideEncryption> {
    let valid_algorithm = text_setting("s3_server_side_encryption", algorithm);
    let valid_key_id = text_setting("s3_sse_kms_key_id", kms_key_id);
    let chosen = valid_algorithm
        .filter(|text| !text.is_empty())
        .or_else(|| {
            valid_key_id
                .as_ref()
                .filter(|key| !key.is_empty())
                .map(|_| "aws:kms".to_owned())
        })?;
    let kms_key = valid_key_id.filter(|key| !key.is_empty());
    if kms_key.is_some() && !chosen.starts_with("aws:kms") {
        tracing::warn!(
            algorithm = chosen,
            "s3 logging: ignoring s3_sse_kms_key_id because s3_server_side_encryption is not aws:kms"
        );
        return Some(ServerSideEncryption {
            algorithm: chosen,
            kms_key_id: None,
        });
    }
    Some(ServerSideEncryption {
        algorithm: chosen,
        kms_key_id: kms_key,
    })
}

fn text_setting(setting: &str, configured: Option<&OptionValue>) -> Option<String> {
    match configured {
        None => None,
        Some(OptionValue::Text(text)) => Some(text.clone()),
        Some(value) => {
            let kind = value.type_name();
            tracing::warn!(
                setting,
                kind,
                "s3 logging: ignoring an encryption option that is not a string"
            );
            None
        }
    }
}
