use std::collections::BTreeMap;

use betterlite_config::sections::bedrock_services_s3_access::{
    BUCKET_OWNER_ENV, ENCRYPTION_KEY_ID_ENV,
};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::S3AccessError;

const ENCRYPTION_KEY_ID_PARAM: &str = "s3_encryption_key_id";
const BUCKET_OWNER_PARAM: &str = "s3_bucket_owner";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Text(String),
    Other,
}

pub type SettingMap = BTreeMap<String, SettingValue>;

#[derive(Debug, Clone, Copy)]
pub enum S3Setting {
    EncryptionKeyId,
    BucketOwner,
}

pub struct S3SettingSources<'sources> {
    pub params: &'sources SettingMap,
    pub optional_params: &'sources SettingMap,
    pub secrets: &'sources SecretLookup,
    pub environment: &'sources BTreeMap<String, String>,
}

pub fn resolve_s3_setting(
    setting: S3Setting,
    sources: &S3SettingSources<'_>,
) -> Result<Option<SecretString>, S3AccessError> {
    let (name, variable) = match setting {
        S3Setting::EncryptionKeyId => (ENCRYPTION_KEY_ID_PARAM, ENCRYPTION_KEY_ID_ENV),
        S3Setting::BucketOwner => (BUCKET_OWNER_PARAM, BUCKET_OWNER_ENV),
    };
    for map in [sources.params, sources.optional_params] {
        if let Some(SettingValue::Text(value)) = map.get(name)
            && !value.is_empty()
        {
            return Ok(Some(SecretString::from(value.clone())));
        }
    }
    let selected = sources
        .secrets
        .get_secret_str(sources.environment, variable, None)
        .map_err(S3AccessError::Secret)?;
    Ok(selected.filter(|value| !value.expose_secret().is_empty()))
}
